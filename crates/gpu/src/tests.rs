//! Parity with the CPU oracle.
//!
//! Two layers: the binning is checked everywhere by running the fine shader's arithmetic on the
//! CPU over the binned scene (no GPU needed), and the real GPU path is checked against the
//! oracle on the sample scene and analytic scenes when an adapter exists. Without one those
//! tests print why and pass, so CI machines without a GPU stay green.

use super::*;
use crate::binning::{self as bn, Binned, PAINT_VEC4S, TILE, piece_cover, solid_of};
use anicraft_doc::{BlendMode, ColorEffect, ColorTransform, DisplayItem, ElementId, ElementKind, GradientStop, Group, LayerId, LayerKind, MaskMode, Paint, Rgba, Stroke, sample};
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
        color: ColorTransform::IDENTITY,
        layer: LayerId(0),
        element: ElementId(0),
        outline: None,
        clip: None,
        masks: None,
        guide: false,
        group: None,
        image: None,
        smooth: true,
        warp: None,
    }
}

fn list(items: Vec<DisplayItem>) -> DisplayList {
    DisplayList { clip_modes: Vec::new(), items, ..Default::default() }
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
    let mut lin = item(Path::rect(0.0, 0.0, 100.0, 60.0), Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), stops(&[(0.0, red), (0.5, Rgba::WHITE), (1.0, blue)])));
    lin.transform = Affine::translate(70.0, 10.0).mul(&Affine::rotate(0.4)).mul(&Affine::scale(1.2, 0.8));
    let rad = item(Path::ellipse(60.0, 90.0, 50.0, 30.0), Paint::radial(Vec2::new(60.0, 90.0), 50.0, stops(&[(0.0, Rgba::WHITE), (1.0, Rgba::new(0.0, 0.0, 0.0, 0.3))])));
    out.push(("gradients", list(vec![lin, rad]), view(180, 130, Some(Rgba::new(0.9, 0.9, 0.85, 1.0)))));
    // Gradient options (ADR 0074): reflect and repeat, linear light, a focal point on a turned
    // ellipse that repeats.
    let three = stops(&[(0.0, red), (0.5, Rgba::new(1.0, 0.9, 0.3, 1.0)), (1.0, blue)]);
    let mut refl = Paint::linear(Vec2::new(20.0, 0.0), Vec2::new(50.0, 0.0), three.clone());
    if let Paint::Linear { spread, linear_rgb, .. } = &mut refl {
        *spread = anicraft_doc::Spread::Reflect;
        *linear_rgb = true;
    }
    let mut rep = Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(14.0, 9.0), three.clone());
    if let Paint::Linear { spread, .. } = &mut rep {
        *spread = anicraft_doc::Spread::Repeat;
    }
    let mut foc = Paint::radial(Vec2::ZERO, 1.0, three).with_frame(&Affine::translate(140.0, 60.0).mul(&Affine::rotate(0.6)).mul(&Affine::scale(40.0, 20.0)), None);
    if let Paint::Radial { focal, spread, .. } = &mut foc {
        *focal = 0.6;
        *spread = anicraft_doc::Spread::Repeat;
    }
    let mut rep_item = item(Path::rect(0.0, 60.0, 90.0, 50.0), rep);
    rep_item.transform = Affine::translate(5.0, 0.0);
    out.push(("gradient options", list(vec![item(Path::rect(0.0, 0.0, 90.0, 55.0), refl), rep_item, item(Path::ellipse(140.0, 60.0, 60.0, 50.0), foc)]), view(210, 120, Some(Rgba::WHITE))));
    // Strokes: wide, thin (sub-pixel, faded), on open and closed paths.
    let mut wide = item(Path::line(Vec2::new(10.0, 20.0), Vec2::new(150.0, 70.0)), Paint::Solid(red));
    wide.fill = None;
    wide.stroke = Some(Stroke::new(Rgba::BLACK, 9.0));
    let mut thin = item(Path::ellipse(80.0, 50.0, 60.0, 30.0), Paint::Solid(Rgba::new(1.0, 0.9, 0.2, 1.0)));
    thin.stroke = Some(Stroke::new(Rgba::new(0.1, 0.1, 0.4, 1.0), 0.4));
    out.push(("strokes", list(vec![thin, wide]), view(160, 100, Some(Rgba::WHITE))));
    // Variable-width strokes (ADR 0018) come prepared by the oracle, so the GPU draws them alike.
    let mut taper = item(Path::line(Vec2::new(10.0, 80.0), Vec2::new(150.0, 15.0)), Paint::Solid(red));
    taper.fill = None;
    taper.stroke = Some(Stroke::new(Rgba::BLACK, 12.0).with_profile(anicraft_doc::ProfilePreset::TaperBoth.profile()));
    let mut bulge = item(Path::ellipse(80.0, 50.0, 55.0, 28.0), Paint::Solid(Rgba::new(0.3, 0.8, 0.5, 1.0)));
    bulge.stroke = Some(Stroke::new(Rgba::new(0.1, 0.1, 0.4, 1.0), 8.0).with_profile(anicraft_doc::ProfilePreset::Bulge.profile()));
    out.push(("variable-width strokes", list(vec![bulge, taper]), view(160, 100, Some(Rgba::WHITE))));
    // Caps, joins and dashes (ADR 0046) also come prepared by the oracle.
    use anicraft_doc::{Cap, Dash, Join, StrokeStyle};
    let mut zag = Path::new();
    zag.move_to(Vec2::new(12.0, 80.0));
    for p in [Vec2::new(45.0, 20.0), Vec2::new(78.0, 80.0), Vec2::new(111.0, 20.0)] {
        zag.line_to(p);
    }
    let mut zig = item(zag, Paint::Solid(red));
    zig.fill = None;
    zig.stroke = Some(Stroke::new(Rgba::BLACK, 10.0).with_style(StrokeStyle { cap: Cap::Square, join: Join::Miter, miter_limit: 4.0, dash: None }));
    let mut dashed = item(Path::ellipse(90.0, 50.0, 55.0, 30.0), Paint::Solid(Rgba::new(1.0, 0.9, 0.2, 1.0)));
    dashed.stroke = Some(Stroke::new(Rgba::new(0.1, 0.1, 0.4, 1.0), 4.0).with_style(StrokeStyle { cap: Cap::Butt, join: Join::Bevel, dash: Dash::dashed(9.0, 5.0), ..StrokeStyle::default() }));
    let mut dots = item(Path::line(Vec2::new(10.0, 92.0), Vec2::new(150.0, 92.0)), Paint::Solid(red));
    dots.fill = None;
    dots.stroke = Some(Stroke::new(red, 5.0).with_profile(anicraft_doc::ProfilePreset::TaperEnd.profile()).with_style(StrokeStyle { dash: Dash::dotted(3.0), ..StrokeStyle::default() }));
    out.push(("styled strokes", list(vec![dashed, zig, dots]), view(160, 100, Some(Rgba::WHITE))));
    // Larger than the surface on every side; colour effects (onion-skin style tint and alpha).
    let mut huge = item(Path::ellipse(50.0, 40.0, 300.0, 200.0), Paint::Solid(red));
    huge.color = ColorTransform::alpha(0.5).concat(&ColorTransform::tint(Rgba::hex(0x4cd07d), 0.7));
    out.push(("off-surface", list(vec![huge]), view(100, 80, Some(Rgba::WHITE))));
    out.push(("outlines", outlined(), view(150, 110, Some(Rgba::WHITE))));
    out.extend(composited_scenes());
    out
}

/// Outline mode (ADR 0048): hairlines along closed and open paths, crossing tiles in every
/// direction, overlapping, faded, past the surface, over a fill, and a point.
fn outlined() -> DisplayList {
    let outline = |path: Path, c: Rgba, t: Affine| {
        let mut i = item(path, Paint::Solid(Rgba::BLACK));
        i.outline = Some(c);
        i.transform = t;
        i
    };
    let blue = Rgba::hex(0x4f8dff).with_alpha(0.5);
    let green = Rgba::hex(0x4cd07d).with_alpha(0.8);
    let mut wave = Path::default();
    wave.move_to(Vec2::new(-20.0, 90.0));
    for k in 0..12 {
        wave.line_to(Vec2::new(k as f32 * 15.3, 90.0 + if k % 2 == 0 { 9.7 } else { -7.1 }));
    }
    list(vec![
        item(Path::rect(20.0, 20.0, 50.0, 40.0), Paint::Solid(Rgba::new(0.9, 0.8, 0.2, 1.0))),
        outline(Path::ellipse(50.0, 45.0, 37.3, 29.1), blue, Affine::IDENTITY),
        outline(Path::star(95.0, 50.0, 40.0, 16.0, 5), green, Affine::rotate(0.05)),
        outline(Path::star(95.0, 50.0, 40.0, 16.0, 5), green, Affine::translate(0.25, 0.5)),
        outline(Path::rect(16.0, 32.0, 112.0, 0.0), Rgba::BLACK, Affine::IDENTITY),
        outline(Path::rect(8.5, 4.25, 133.0, 100.0), Rgba::new(0.8, 0.1, 0.1, 1.0), Affine::IDENTITY),
        outline(wave, Rgba::new(0.2, 0.2, 0.2, 0.9), Affine::IDENTITY),
        outline(Path::ellipse(140.0, 100.0, 60.0, 50.0), blue, Affine::IDENTITY),
        outline(Path::line(Vec2::new(60.5, 70.5), Vec2::new(60.5, 70.5)), Rgba::BLACK, Affine::IDENTITY),
    ])
}

/// Masks and groups (ADR 0029): each stresses one part of the oracle's band compositor.
fn composited_scenes() -> Vec<(&'static str, DisplayList, View)> {
    let red = Rgba::new(0.9, 0.1, 0.1, 1.0);
    let blue = Rgba::new(0.1, 0.2, 0.9, 0.6);
    let green = Rgba::new(0.2, 0.7, 0.3, 0.8);
    let grad = || Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(120.0, 40.0), stops(&[(0.0, red), (0.5, Rgba::new(1.0, 0.9, 0.3, 1.0)), (1.0, blue)]));
    let mut out = Vec::new();
    // A mask: a gradient seen through a circle, an unclipped shape over it, and one under it.
    let dl = DisplayList {
        clip_modes: Vec::new(),
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
    out.push(("nested masks", DisplayList { clip_modes: Vec::new(), items, clip_parents: vec![None, Some(0)], groups: Vec::new() }, view(130, 110, Some(Rgba::new(0.95, 0.95, 0.9, 1.0)))));
    // Group alpha: overlapping shapes fade as one; nested groups multiply; a group spans tiles
    // and the render bands; a top-level shape between two groups.
    let dl = DisplayList {
        clip_modes: Vec::new(),
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
    out.push(("blend modes", DisplayList { clip_modes: Vec::new(), items, clip_parents: Vec::new(), groups }, view(14 * 24, 96, Some(Rgba::new(0.3, 0.3, 0.5, 0.8)))));
    // Masks inside groups and groups inside masks, with a blend mode, on transparent.
    let dl = DisplayList {
        clip_modes: Vec::new(),
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
        clip_modes: Vec::new(),
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
    // Soft masks (ADR 0070): an alpha gradient fading a shape out, a luminance mask with a
    // stroke and overlapping faded art, and a soft mask inside a coverage one.
    let fade = Paint::linear(Vec2::new(10.0, 0.0), Vec2::new(110.0, 0.0), stops(&[(0.0, Rgba::new(1.0, 1.0, 1.0, 0.0)), (1.0, Rgba::new(0.2, 0.9, 0.4, 1.0))]));
    let soft_art = |path: Path, paint: Paint, clip: u32| {
        let mut m = item(path, paint);
        m.masks = Some(clip);
        m
    };
    let mut ring = soft_art(Path::ellipse(60.0, 50.0, 30.0, 26.0), Paint::Solid(Rgba::new(0.5, 0.5, 0.5, 0.7)), 1);
    ring.stroke = Some(Stroke::new(Rgba::WHITE, 6.0));
    let mut faded = soft_art(Path::rect(50.0, 10.0, 60.0, 30.0), Paint::Solid(Rgba::new(0.9, 0.2, 0.1, 1.0)), 1);
    faded.color = ColorTransform::alpha(0.6);
    let mut inner = soft_art(Path::rect(0.0, 0.0, 120.0, 100.0), Paint::Solid(Rgba::new(1.0, 1.0, 1.0, 0.5)), 3);
    inner.clip = Some(2);
    let mut dl = DisplayList {
        items: vec![
            item(Path::rect(0.0, 0.0, 120.0, 100.0), Paint::Solid(Rgba::new(0.95, 0.9, 0.8, 1.0))),
            clipped(item(Path::rect(0.0, 0.0, 120.0, 50.0), grad()), 0),
            soft_art(Path::rect(10.0, 0.0, 100.0, 60.0), fade, 0),
            clipped(item(Path::rect(0.0, 30.0, 120.0, 70.0), Paint::Solid(red)), 1),
            ring,
            faded,
            clipped(item(Path::star(60.0, 60.0, 40.0, 15.0, 5), Paint::Solid(blue)), 3),
            mask(Path::rect(20.0, 40.0, 50.0, 60.0), 2),
            inner,
        ],
        clip_parents: vec![None, None, None, Some(2)],
        ..Default::default()
    };
    dl.set_clip_mode(0, MaskMode::Alpha);
    dl.set_clip_mode(1, MaskMode::Luminance);
    dl.set_clip_mode(3, MaskMode::Alpha);
    out.push(("soft masks", dl, view(120, 100, Some(Rgba::WHITE))));
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
            let (mut level_mode, mut level_colour) = (0u32, [0.0f32; 4]);
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
                        level_mode = b.index[e + 1];
                        level_colour = [0.0; 4];
                        continue;
                    }
                    bn::LEVEL_END => {
                        let mode = [MaskMode::Coverage, MaskMode::Alpha, MaskMode::Luminance][level_mode as usize];
                        mask *= if mode.is_soft() { mode.reveal(level_colour) } else { level };
                        continue;
                    }
                    k => assert_eq!(k & bn::CTRL, 0, "unknown control entry {k:x}"),
                }
                let hd = op * 4;
                let mut cov = entry_cover(&b.geo[first..first + n], bd, b.op_heads[hd + 1], lx, ly);
                let flags = b.op_heads[hd + 3];
                let shade = || match b.op_heads[hd] {
                    0 => solid_of(b, op),
                    kind => {
                        let base = op * PAINT_VEC4S;
                        let (m, nn, g) = (b.paints[base + 1], b.paints[base + 2], b.paints[base + 3]);
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let q = Vec2::new(m[0] * p.x + m[2] * p.y + nn[0], m[1] * p.x + m[3] * p.y + nn[1]);
                        let d = q - Vec2::new(nn[2], nn[3]);
                        // As fine.wgsl: spread code and focal point in the first vec4.
                        let o = b.paints[base];
                        let t = if kind == 1 { d.x * g[0] + d.y * g[1] } else { anicraft_render::paint::focal_t(d * g[2], o[1]) };
                        let t = match o[0] as u32 {
                            1 => anicraft_doc::Spread::Reflect.apply(t),
                            2 => anicraft_doc::Spread::Repeat.apply(t),
                            _ => t,
                        };
                        let i = (t.clamp(0.0, 1.0) * 255.0 + 0.5) as usize & 255;
                        b.paints[b.op_heads[hd + 2] as usize + i]
                    }
                };
                if flags & bn::ART != 0 {
                    if level_mode != 0 {
                        if cov > 1e-4 {
                            let s = shade();
                            let sa = s[3] * cov;
                            for k in 0..4 {
                                level_colour[k] = s[k] * cov + level_colour[k] * (1.0 - sa);
                            }
                        }
                        continue;
                    }
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
                let s = shade();
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

/// An entry's coverage of pixel `(lx, ly)` of its tile, as `fine.wgsl` computes it: the
/// backdrop and its pieces under fill rule `rule`, or a hairline's distance.
fn entry_cover(pieces: &[[f32; 4]], backdrop: u32, rule: u32, lx: usize, ly: usize) -> f32 {
    if rule == bn::HAIRLINE {
        let p = Vec2::new(lx as f32 + 0.5, ly as f32 + 0.5);
        let d = pieces.iter().fold(2.0f32, |d, s| d.min(anicraft_render::hairline::seg_dist(p, Vec2::new(s[0], s[1]), Vec2::new(s[2], s[3]))));
        return (1.0 - d).clamp(0.0, 1.0);
    }
    let mut wnd = backdrop as i32 as f32;
    for s in pieces {
        wnd += piece_cover(*s, lx as f32, ly as f32);
    }
    if rule == 0 {
        wnd.abs().min(1.0)
    } else {
        let t = wnd.abs() % 2.0;
        if t > 1.0 { 2.0 - t } else { t }
    }
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
    let cam = anicraft_doc::Camera { center: Vec2::new(700.0, 380.0), zoom: 1.6, rotation: -0.15, color: anicraft_doc::ColorEffect::Tint { color: Rgba::hex(0x3366ff), amount: 0.3 } };
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
                        e.color = e.color.with_alpha(if i == 0 { 0.6 } else { 0.5 });
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

/// A rounded panel with 9-slice guides, placed stretched, scaled up and turned.
fn nine_slice_sample() -> anicraft_doc::Document {
    use anicraft_doc::{Element, ElementId, ElementKind, Keyframe, Layer, LayerId, ScaleGrid, Shape, Stroke, Symbol, SymbolId, SymbolKind, Timeline};
    let mut d = anicraft_doc::Document { stage: anicraft_doc::Stage { width: 320, height: 200, ..Default::default() }, ..Default::default() };
    let mut shape = Shape::filled(Path::rounded_rect(0.0, 0.0, 60.0, 30.0, 8.0), Paint::Solid(Rgba::hex(0x2a9d8f)));
    shape.stroke = Some(Stroke::new(Rgba::hex(0x264653), 2.0));
    let el = |id, transform, kind| std::sync::Arc::new(Element { id: ElementId(id), name: String::new(), transform, color: ColorEffect::default(), kind });
    let mut k = Keyframe::new(0, 1);
    k.elements.push(el(2, Affine::IDENTITY, ElementKind::Shape(shape)));
    let mut l = Layer::new(LayerId(3), "art");
    l.keyframes = vec![std::sync::Arc::new(k)];
    d.library.push(std::sync::Arc::new(Symbol { id: SymbolId(4), name: "Panel".into(), kind: SymbolKind::MovieClip, linkage: None, scale_grid_kept: None, scale_grid: Some(ScaleGrid { left: 8.0, right: 52.0, top: 8.0, bottom: 22.0 }), timeline: Timeline { layers: vec![std::sync::Arc::new(l)] } }));
    let inst = |id, xf| el(id, xf, ElementKind::Symbol { symbol: SymbolId(4), loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() });
    let mut k = Keyframe::new(0, 1);
    k.elements.push(inst(5, Affine::translate(10.0, 10.0).mul(&Affine::scale(4.5, 1.2))));
    k.elements.push(inst(6, Affine::translate(30.0, 70.0).mul(&Affine::scale(2.0, 3.5))));
    k.elements.push(inst(7, Affine::translate(220.0, 90.0).mul(&Affine::rotate(0.5)).mul(&Affine::scale(1.5, 2.5))));
    std::sync::Arc::make_mut(&mut d.timeline_mut(0).layers[0]).keyframes = vec![std::sync::Arc::new(k)];
    d
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
    // A 9-sliced instance (ADR 0055) reaches the stage through evaluation: cut outlines, strokes
    // at their own width.
    let nine = nine_slice_sample();
    all.push(("9-slice", anicraft_doc::evaluate(&nine, 0), View::stage(&nine, 0.7)));
    for (name, dl, v) in all {
        let (scene, _) = prepare_scene(&dl, &v).unwrap();
        let b = binning::bin(&scene, v.width, v.height).unwrap();
        let (oracle, _) = render(&dl, &v).unwrap();
        let (mean, max) = f32_diff(&emulate(&b, &v), &oracle.data);
        assert!(mean < 1e-4 && max < 2e-3, "{name}: mean {mean:e}, max {max:e}");
    }
}

/// Thousands of overlapping shapes at awkward scales: edges meet tile corners and boundaries
/// within float rounding, where the backdrops and the left-edge steps must agree (ADR 0048).
#[test]
fn binned_coverage_matches_the_oracle_on_stress_frames() {
    let doc = sample::stress(20, 100, 48);
    for (f, s, at) in [(0u32, 1.0f32, (0.0, 0.0)), (30, 0.5, (0.0, 0.0)), (7, 2.3, (-900.0, -700.0)), (41, 0.73, (-11.5, -3.25))] {
        let dl = anicraft_doc::evaluate(&doc, f);
        let v = View { transform: Affine::translate(at.0, at.1).mul(&Affine::scale(s, s)), width: 1920, height: 1080, background: Some(Rgba::WHITE) };
        let (scene, _) = prepare_scene(&dl, &v).unwrap();
        let b = binning::bin(&scene, v.width, v.height).unwrap();
        let (oracle, _) = render(&dl, &v).unwrap();
        let (mean, max) = f32_diff(&emulate(&b, &v), &oracle.data);
        assert!(mean < 1e-4 && max < 4e-3, "frame {f} at {s}: mean {mean:e}, max {max:e}");
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
        clip_modes: Vec::new(),
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
        clip_modes: Vec::new(),
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
        DisplayList { clip_modes: Vec::new(), items, clip_parents: (0..n).map(|i| i.checked_sub(1).map(|p| p as u32)).collect(), groups: Vec::new() }
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
        layer: None,
        hairline: false,
    };
    // A fill naming a layer that isn't there draws nothing.
    let mut lost = fill(None, None);
    lost.layer = Some(3);
    let scene = anicraft_render::Scene { fills: vec![fill(None, None), fill(Some(7), Some(9)), lost], clips: Vec::new(), groups: Vec::new(), layers: Vec::new() };
    let b = binning::bin(&scene, 32, 32).unwrap();
    assert!(b.geo.iter().flatten().all(|v| v.is_finite()));
}

// ----- the real GPU ----------------------------------------------------------------------------

/// The shaders, as `pipelines` and the stage build them.
const SHADERS: [(&str, &str); 4] = [
    ("fine.wgsl", include_str!("fine.wgsl")),
    ("bin.wgsl", include_str!("bin.wgsl")),
    ("scan.wgsl", include_str!("scan.wgsl")),
    ("filters.wgsl", include_str!("filters.wgsl")),
];

#[test]
fn shaders_take_no_signed_remainder() {
    // naga writes WGSL's `%` on integers as GLSL's, which is undefined when an operand is
    // negative: on the GL backend (llvmpipe in the FreeBSD VM) a repeating bitmap's wrap,
    // `((q % size) + size) % size` at q = -1, picked the wrong texel at its edges (bitmap
    // repeated: max 79). Unsigned remainders and float ones (written as `a - b * trunc(a / b)`)
    // are defined on every backend.
    for (name, src) in SHADERS {
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let functions = module.functions.iter().map(|(h, f)| (f, &info[h]));
        let entries = module.entry_points.iter().enumerate().map(|(i, e)| (&e.function, info.get_entry_point(i)));
        for (f, fi) in functions.chain(entries) {
            for (h, e) in f.expressions.iter() {
                if let naga::Expression::Binary { op: naga::BinaryOperator::Modulo, left, .. } = *e {
                    let kind = fi[left].ty.inner_with(&module.types).scalar_kind();
                    assert_ne!(kind, Some(naga::ScalarKind::Sint), "{name}: `%` on a signed integer in {:?} ({h:?})", f.name);
                }
            }
        }
    }
}

/// One device for all tests: the graphics card, or else the software adapter (CI runners have
/// lavapipe or WARP); `None` (with the reasons printed) when there is neither.
/// `ANICRAFT_TEST_GPU=software` tests on the software adapter even where there is a card.
fn gpu() -> Option<&'static GpuRenderer> {
    static GPU: OnceLock<Option<GpuRenderer>> = OnceLock::new();
    GPU.get_or_init(|| {
        let card = if std::env::var("ANICRAFT_TEST_GPU").is_ok_and(|v| v == "software") { Err(GpuError::Unavailable("software asked for".into())) } else { GpuRenderer::new() };
        match card.or_else(|e| GpuRenderer::software().map_err(|s| format!("{e}; software: {s}"))) {
            Ok(g) => {
                eprintln!("GPU parity tests on {}", g.adapter);
                Some(g)
            }
            Err(e) => {
                eprintln!("skipping GPU parity tests: {e}");
                None
            }
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
    // Filtered symbols nested deeper than MAX_FILTER_DEPTH are the CPU's; one less is the GPU's.
    let nested = |depth: usize| {
        let groups: Vec<Group> = (0..depth)
            .map(|i| {
                let mut grp = group(i.checked_sub(1).map(|p| p as u32), 1.0, BlendMode::Normal);
                grp.filters = Arc::from(vec![anicraft_doc::Filter::Blur { blur_x: 2.0, blur_y: 2.0, quality: 1 }]);
                grp
            })
            .collect();
        let mut it = in_group(item(Path::rect(2.0, 2.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK)), depth as u32 - 1);
        it.color = ColorTransform::alpha(0.5);
        DisplayList { clip_modes: Vec::new(), items: vec![it], clip_parents: Vec::new(), groups }
    };
    let deep = nested(MAX_FILTER_DEPTH + 1);
    assert!(matches!(g.render(&deep, &view(12, 12, None), Output::Premul8), Err(GpuError::NeedsFull(NeedsFull::Filters))));
    assert!(matches!(g.render_texture(&deep, &view(12, 12, None)), Err(GpuError::NeedsFull(NeedsFull::Filters))));
    assert_parity(g, "filters nested as deep as the GPU draws", &nested(MAX_FILTER_DEPTH), &view(12, 12, Some(Rgba::WHITE)));
    assert!(matches!(g.render(&list(vec![]), &view(0, 8, None), Output::Premul8), Err(GpuError::TooLarge(_))));
    assert!(matches!(g.render(&list(vec![]), &view(100_000, 100_000, None), Output::Premul8), Err(GpuError::TooLarge(_))));
    assert!(matches!(g.render_texture(&list(vec![]), &view(0, 8, None)), Err(GpuError::TooLarge(_))));
    // An empty list is just the background; and the device is still fine afterwards.
    let f = g.render(&list(vec![]), &view(20, 3, Some(Rgba::new(0.0, 1.0, 0.0, 1.0))), Output::Premul8).unwrap();
    assert!(f.premul8.as_chunks::<4>().0.iter().all(|p| *p == [0, 255, 0, 255]));
    assert!(g.broken().is_none());
}

/// ADR 0092: a device lost mid-session (here destroyed on purpose, the way a driver reset
/// takes it) never panics: every later call is an error saying why, so the stage hands the
/// frame to the CPU. A renderer told from outside (the window's device was lost) refuses too.
#[test]
fn a_lost_device_turns_every_call_into_an_error_and_says_why() {
    // A device of its own: the shared one stays for the other tests.
    let opened = GpuRenderer::new().or_else(|_| GpuRenderer::software());
    let Ok(g) = opened else {
        eprintln!("skipping the lost-device test: no GPU");
        return;
    };
    let dl = list(vec![item(Path::rect(1.0, 1.0, 6.0, 6.0), Paint::Solid(Rgba::BLACK))]);
    assert!(g.render(&dl, &view(8, 8, None), Output::Premul8).is_ok());
    g.device.destroy();
    let _ = g.wait_idle();
    let first = g.render(&dl, &view(8, 8, None), Output::Premul8);
    assert!(matches!(first, Err(GpuError::Device(_))), "a lost device fails the frame");
    assert!(g.broken().is_some(), "and the renderer says it is broken");
    assert!(matches!(g.render_texture(&dl, &view(8, 8, None)), Err(GpuError::Device(_))));
    // Told from outside: the first reason stays.
    let why = g.broken();
    g.fail("the window's device was lost");
    assert_eq!(g.broken(), why);
    // A fresh renderer told from outside refuses at once, with that reason.
    if let Ok(h) = GpuRenderer::new().or_else(|_| GpuRenderer::software()) {
        h.fail("the window's device was lost");
        match h.render(&dl, &view(8, 8, None), Output::Premul8) {
            Err(GpuError::Device(e)) => assert!(e.contains("window's device"), "{e}"),
            other => panic!("expected a device error, got {:?}", other.map(|f| f.width)),
        }
    }
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

// ----- bitmaps and filters (ADR 0032) ---------------------------------------------------------

/// A `w × h` test image: a hue ramp with an alpha ramp, so every tap and every seam differs.
fn pixels(w: u32, h: u32, seed: u32) -> Arc<anicraft_doc::Pixels> {
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let v = x.wrapping_mul(37).wrapping_add(y.wrapping_mul(91)).wrapping_add(seed.wrapping_mul(53));
            rgba.extend_from_slice(&[(v % 256) as u8, ((v / 3 + 80) % 256) as u8, ((x * 255) / w.max(1)) as u8, (255 - (y * 200) / h.max(1)) as u8]);
        }
    }
    Arc::new(anicraft_doc::Pixels { width: w, height: h, rgba })
}

fn bitmap_item(path: Path, px: &Arc<anicraft_doc::Pixels>, repeat: bool, matrix: Affine) -> DisplayItem {
    let mut i = item(path, Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat, matrix });
    i.image = Some(px.clone());
    i
}

/// Bitmap fills: placed, scaled, turned and repeated; with a colour effect; clipped and
/// grouped; tiny and malformed images.
fn bitmap_scenes() -> Vec<(&'static str, DisplayList, View)> {
    let a = pixels(13, 9, 1);
    let b = pixels(1, 1, 2);
    let c = pixels(40, 30, 3);
    let white = Some(Rgba::WHITE);
    let turned = Affine::translate(30.0, 12.0).mul(&Affine::rotate(0.4)).mul(&Affine::scale(3.3, 2.1));
    let mut out = vec![
        ("bitmap placed once", list(vec![bitmap_item(Path::rect(2.0, 3.0, 90.0, 70.0), &a, false, turned)]), view(100, 80, white)),
        ("bitmap repeated", list(vec![bitmap_item(Path::ellipse(50.0, 40.0, 45.0, 35.0), &a, true, turned)]), view(100, 80, None)),
        ("bitmap magnified, mirrored", list(vec![bitmap_item(Path::rect(0.0, 0.0, 64.0, 48.0), &c, false, Affine { m: [-1.7, 0.0, 0.0, 1.6, 66.0, -1.3] })]), view(64, 48, white)),
        ("one-pixel bitmap", list(vec![bitmap_item(Path::rect(5.5, 5.5, 20.0, 20.0), &b, true, Affine::scale(7.0, 7.0))]), view(32, 32, white)),
    ];
    // Colour effects: tint and alpha per tap, and a NaN multiplier (a channel of 0, as the oracle
    // draws it).
    let mut tinted = bitmap_item(Path::rect(0.0, 0.0, 60.0, 40.0), &c, true, Affine::scale(0.7, 0.9));
    tinted.color = ColorTransform::alpha(0.6).concat(&ColorTransform::tint(Rgba::new(0.2, 0.9, 0.1, 1.0), 0.4));
    let mut nan = bitmap_item(Path::rect(30.0, 10.0, 30.0, 30.0), &a, false, Affine::scale(2.0, 2.0));
    nan.color = ColorTransform { mul: [f32::NAN, 0.7, 0.7, 1.0], add: [0.0, 0.15, 0.15, 0.0] };
    // Advanced (ADR 0056): a negative multiplier, offsets and an alpha offset.
    let mut advanced = bitmap_item(Path::rect(4.0, 20.0, 40.0, 26.0), &a, true, Affine::scale(1.3, 1.1));
    advanced.color = ColorTransform { mul: [-1.0, 0.5, 0.2, 0.6], add: [1.0, 0.1, 0.4, 0.2] };
    out.push(("bitmap colour effects", list(vec![tinted, nan, advanced]), view(64, 48, Some(Rgba::new(0.1, 0.1, 0.3, 1.0)))));
    // A short (malformed) buffer: transparent past its end; an empty image draws nothing.
    let short = Arc::new(anicraft_doc::Pixels { width: 4, height: 4, rgba: vec![200; 20] });
    let empty = Arc::new(anicraft_doc::Pixels { width: 0, height: 3, rgba: Vec::new() });
    out.push(("malformed bitmaps", list(vec![bitmap_item(Path::rect(0.0, 0.0, 30.0, 30.0), &short, false, Affine::scale(7.0, 7.0)), bitmap_item(Path::rect(10.0, 0.0, 10.0, 30.0), &empty, true, Affine::IDENTITY)]), view(32, 32, white)));
    // Inside a mask and a faded group, over a gradient.
    let grad = item(Path::rect(0.0, 0.0, 80.0, 60.0), Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(80.0, 60.0), stops(&[(0.0, Rgba::BLACK), (1.0, Rgba::new(1.0, 0.8, 0.0, 1.0))])));
    let inside = in_group(clipped(bitmap_item(Path::rect(4.0, 4.0, 72.0, 52.0), &a, true, turned), 0), 0);
    let dl = DisplayList { clip_modes: Vec::new(), items: vec![grad, inside, mask(Path::ellipse(40.0, 30.0, 30.0, 22.0), 0)], clip_parents: vec![None], groups: vec![group(None, 0.7, BlendMode::Multiply)] };
    out.push(("bitmap masked and multiplied", dl, view(80, 60, white)));
    // Without smoothing (ADR 0050): the nearest pixel, magnified, turned and repeated.
    let mut hard = bitmap_item(Path::rect(2.0, 3.0, 90.0, 70.0), &a, false, turned);
    hard.smooth = false;
    let mut hard_tiled = bitmap_item(Path::ellipse(50.0, 40.0, 45.0, 35.0), &c, true, Affine::scale(4.0, 3.0));
    hard_tiled.smooth = false;
    out.push(("bitmaps without smoothing", list(vec![hard_tiled, hard]), view(100, 80, white)));
    // Warped (ADR 0079): bent by three pins, folded a little, faded; the oracle's prepared image.
    use anicraft_doc::warp::{Pin, PinId, Warp};
    let pin = |id: u64, rest: (f32, f32), at: (f32, f32)| Pin { id: PinId(id), rest: Vec2::new(rest.0, rest.1), at: Vec2::new(at.0, at.1) };
    let mut warp = Warp::new(Arc::new(Path::rect(0.0, 0.0, 40.0, 30.0)), 8);
    warp.pins = vec![pin(1, (2.0, 15.0), (6.0, 20.0)), pin(2, (20.0, 15.0), (24.0, 14.0)), pin(3, (38.0, 15.0), (52.0, 40.0))];
    let mut bent = bitmap_item(warp.outline().as_ref().clone(), &c, false, Affine::IDENTITY);
    bent.warp = warp.mesh().map(Arc::new);
    bent.transform = Affine::scale(1.4, 1.4);
    bent.color = ColorTransform::alpha(0.8);
    out.push(("warped bitmap", list(vec![bent]), view(90, 72, white)));
    out
}

#[test]
fn gpu_matches_the_oracle_on_bitmap_fills() {
    let Some(g) = gpu() else { return };
    for (name, dl, v) in bitmap_scenes() {
        let d = assert_parity(g, name, &dl, &v);
        eprintln!("{name}: mean {:.5} / max {} / {} of {} px differ", d.mean, d.max, d.differing, d.pixels);
    }
    // Each image is uploaded once and kept; drawing it again adds nothing.
    let px = pixels(31, 17, 9);
    let dl = list(vec![bitmap_item(Path::rect(0.0, 0.0, 40.0, 40.0), &px, true, Affine::IDENTITY)]);
    g.render(&dl, &view(40, 40, None), Output::Premul8).unwrap();
    let held = g.cached_images();
    g.render(&dl, &view(40, 40, None), Output::Premul8).unwrap();
    assert_eq!(g.cached_images(), held);
    assert!(held.1 <= IMAGE_BUDGET + 4 * 31 * 17);
    // Sixteen images in one picture draw on the GPU; a seventeenth is the CPU's.
    let many: Vec<Arc<anicraft_doc::Pixels>> = (0..17).map(|i| pixels(3, 2, 100 + i)).collect();
    let items = |n: usize| list(many.iter().take(n).enumerate().map(|(i, p)| bitmap_item(Path::rect(i as f32 * 4.0, 0.0, 4.0, 8.0), p, true, Affine::IDENTITY)).collect());
    assert_parity(g, "sixteen bitmaps", &items(16), &view(72, 8, Some(Rgba::WHITE)));
    assert!(matches!(g.render(&items(17), &view(72, 8, None), Output::Premul8), Err(GpuError::NeedsFull(NeedsFull::Bitmaps))));
}

fn filtered_group(filters: Vec<anicraft_doc::Filter>) -> Group {
    Group { parent: None, alpha: 1.0, blend: BlendMode::Normal, filters: Arc::from(filters), clip: None }
}

/// Every filter, its sides and switches, on a small symbol: a red square with a translucent
/// blue circle over it.
fn filters_to_test() -> Vec<(&'static str, anicraft_doc::Filter)> {
    use anicraft_doc::{Filter as F, FilterSide as S};
    let ramp = stops(&[(0.0, Rgba::new(1.0, 1.0, 1.0, 0.0)), (0.4, Rgba::new(0.0, 1.0, 0.3, 0.8)), (1.0, Rgba::new(0.1, 0.0, 1.0, 1.0))]);
    let bevel_ramp = stops(&[(0.0, Rgba::new(1.0, 1.0, 0.8, 1.0)), (0.5, Rgba::new(0.0, 0.0, 0.0, 0.0)), (1.0, Rgba::new(0.2, 0.0, 0.3, 1.0))]);
    let shadow = |inner, knockout, hide_object| F::DropShadow { blur_x: 5.0, blur_y: 8.0, strength: 1.3, quality: 2, angle: 35.0, distance: 6.0, color: Rgba::new(0.1, 0.0, 0.2, 0.8), inner, knockout, hide_object };
    let bevel = |side, knockout| F::Bevel { blur_x: 4.0, blur_y: 4.0, strength: 1.5, quality: 1, angle: 45.0, distance: 3.0, shadow_color: Rgba::new(0.0, 0.0, 0.0, 0.9), highlight_color: Rgba::WHITE, side, knockout };
    vec![
        ("blur", F::Blur { blur_x: 7.0, blur_y: 4.0, quality: 3 }),
        ("blur, odd and one-sided", F::Blur { blur_x: 5.0, blur_y: 0.0, quality: 1 }),
        ("drop shadow", shadow(false, false, false)),
        ("drop shadow, knockout", shadow(false, true, false)),
        ("drop shadow, hide object", shadow(false, false, true)),
        ("inner shadow", shadow(true, false, false)),
        ("inner shadow, knockout", shadow(true, true, false)),
        ("glow", F::Glow { blur_x: 10.0, blur_y: 10.0, strength: 2.0, quality: 2, color: Rgba::new(1.0, 0.8, 0.0, 1.0), inner: false, knockout: false }),
        ("inner glow, knockout", F::Glow { blur_x: 6.0, blur_y: 9.0, strength: 1.0, quality: 3, color: Rgba::new(0.0, 1.0, 1.0, 0.7), inner: true, knockout: true }),
        ("bevel, inner", bevel(S::Inner, false)),
        ("bevel, outer", bevel(S::Outer, false)),
        ("bevel, full, knockout", bevel(S::Full, true)),
        ("gradient bevel", F::GradientBevel { blur_x: 5.0, blur_y: 3.0, strength: 1.0, quality: 2, angle: 120.0, distance: 4.0, stops: bevel_ramp, side: S::Full, knockout: false }),
        ("gradient glow, outer", F::GradientGlow { blur_x: 8.0, blur_y: 6.0, strength: 1.2, quality: 2, angle: 0.0, distance: 3.0, stops: ramp.clone(), side: S::Outer, knockout: false }),
        ("gradient glow, full, knockout", F::GradientGlow { blur_x: 4.0, blur_y: 4.0, strength: 1.0, quality: 1, angle: 210.0, distance: 2.0, stops: ramp, side: S::Full, knockout: true }),
        ("adjust colour", F::AdjustColor { brightness: -20.0, contrast: 35.0, saturation: 60.0, hue: -70.0 }),
    ]
    .into_iter()
    .map(|(n, f)| (n, f.sanitized()))
    .collect()
}

fn symbol_items(g: u32) -> Vec<DisplayItem> {
    vec![
        in_group(item(Path::rect(14.0, 12.0, 30.0, 26.0), Paint::Solid(Rgba::new(0.9, 0.1, 0.1, 1.0))), g),
        in_group(item(Path::ellipse(40.0, 36.0, 14.0, 14.0), Paint::Solid(Rgba::new(0.1, 0.3, 1.0, 0.6))), g),
    ]
}

#[test]
fn gpu_matches_the_oracle_on_every_filter() {
    let Some(g) = gpu() else { return };
    let backdrop = item(Path::rect(0.0, 0.0, 70.0, 60.0), Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(70.0, 0.0), stops(&[(0.0, Rgba::new(0.9, 0.9, 0.8, 1.0)), (1.0, Rgba::new(0.2, 0.4, 0.5, 1.0))])));
    let mut worst: f64 = 0.0;
    for (name, f) in filters_to_test() {
        let mut items = vec![backdrop.clone()];
        items.extend(symbol_items(0));
        let dl = DisplayList { clip_modes: Vec::new(), items, clip_parents: Vec::new(), groups: vec![filtered_group(vec![f])] };
        // At 1:1; zoomed, so sizes scale; and panned so the layer reaches past the surface.
        for v in [view(70, 60, Some(Rgba::WHITE)), View { transform: Affine::scale(1.5, 1.5), width: 105, height: 90, background: None }, View { transform: Affine::translate(-20.0, -15.0), width: 40, height: 33, background: Some(Rgba::WHITE) }] {
            let d = assert_parity(g, name, &dl, &v);
            worst = worst.max(d.mean);
        }
    }
    eprintln!("filters: worst mean {worst:.5}");
}

#[test]
fn gpu_matches_the_oracle_on_filtered_symbols_in_context() {
    let Some(g) = gpu() else { return };
    use anicraft_doc::Filter as F;
    let blur = F::Blur { blur_x: 4.0, blur_y: 4.0, quality: 2 };
    let shadow = F::DropShadow { blur_x: 4.0, blur_y: 4.0, strength: 1.0, quality: 1, angle: 45.0, distance: 5.0, color: Rgba::BLACK, inner: false, knockout: false, hide_object: false };
    let glow = F::Glow { blur_x: 6.0, blur_y: 6.0, strength: 1.5, quality: 1, color: Rgba::new(1.0, 0.9, 0.0, 1.0), inner: false, knockout: false };
    let white = Some(Rgba::WHITE);
    let bg = item(Path::rect(0.0, 0.0, 80.0, 64.0), Paint::Solid(Rgba::new(0.3, 0.6, 0.4, 1.0)));
    let mut scenes: Vec<(&str, DisplayList)> = Vec::new();
    // Several filters in a row, at part opacity, multiplied.
    let mut grp = filtered_group(vec![shadow.clone(), glow.clone(), F::AdjustColor { brightness: 10.0, contrast: 0.0, saturation: -40.0, hue: 25.0 }]);
    (grp.alpha, grp.blend) = (0.7, BlendMode::Multiply);
    let mut items = vec![bg.clone()];
    items.extend(symbol_items(0));
    scenes.push(("filters in a row, faded and multiplied", DisplayList { clip_modes: Vec::new(), items, clip_parents: Vec::new(), groups: vec![grp] }));
    // A filtered symbol inside a filtered symbol, inside a faded one.
    let mut outer = filtered_group(vec![shadow.clone()]);
    outer.parent = Some(0);
    let mut inner = filtered_group(vec![blur.clone()]);
    inner.parent = Some(1);
    let mut items = vec![bg.clone(), in_group(item(Path::rect(4.0, 40.0, 30.0, 16.0), Paint::Solid(Rgba::new(0.9, 0.6, 0.0, 1.0))), 1)];
    items.extend(symbol_items(2));
    scenes.push(("nested filtered symbols", DisplayList { clip_modes: Vec::new(), items, clip_parents: Vec::new(), groups: vec![group(None, 0.8, BlendMode::Normal), outer, inner] }));
    // Masked from outside (after filtering) and inside (before).
    let mut clipped_grp = filtered_group(vec![glow.clone()]);
    clipped_grp.clip = Some(0);
    let mut items = vec![bg.clone()];
    items.extend(symbol_items(0).into_iter().map(|i| clipped(i, 0)));
    items.push(mask(Path::ellipse(36.0, 30.0, 24.0, 20.0), 0));
    scenes.push(("masked after filtering", DisplayList { clip_modes: Vec::new(), items, clip_parents: vec![None], groups: vec![clipped_grp] }));
    let mut items = vec![bg.clone()];
    items.extend(symbol_items(0).into_iter().map(|i| clipped(i, 0)));
    items.push(in_group(mask(Path::rect(20.0, 10.0, 16.0, 40.0), 0), 0));
    scenes.push(("masked inside the symbol", DisplayList { clip_modes: Vec::new(), items, clip_parents: vec![None], groups: vec![filtered_group(vec![shadow.clone()])] }));
    // A bitmap inside a filtered symbol.
    let px = pixels(9, 7, 4);
    let items = vec![bg.clone(), in_group(bitmap_item(Path::ellipse(40.0, 32.0, 26.0, 20.0), &px, true, Affine::scale(4.0, 4.0)), 0)];
    scenes.push(("filtered bitmap", DisplayList { clip_modes: Vec::new(), items, clip_parents: Vec::new(), groups: vec![filtered_group(vec![blur, glow])] }));
    for (name, dl) in scenes {
        for v in [view(80, 64, white), View { transform: Affine::translate(-37.0, -5.0).mul(&Affine::scale(1.3, 1.3)), width: 48, height: 48, background: white }] {
            let d = assert_parity(g, name, &dl, &v);
            eprintln!("{name}: mean {:.5} / max {} / {} of {} px differ", d.mean, d.max, d.differing, d.pixels);
        }
    }
}

/// The composited sample with what ADR 0032 adds, as artists use it: the ball casts a drop
/// shadow and is bevelled, the star glows, and a bitmap fills the sky.
fn filtered_sample() -> anicraft_doc::Document {
    use anicraft_doc::Filter as F;
    let mut doc = composited_sample();
    let png = anicraft_render::encode_png(48, 32, &pixels(48, 32, 7).rgba).unwrap();
    let bm = anicraft_doc::Bitmap::from_png(anicraft_doc::BitmapId(77), "sky", png).unwrap();
    doc.bitmaps.push(Arc::new(bm));
    let tl = doc.timeline_mut(0);
    for (i, layer) in tl.layers.iter_mut().enumerate() {
        let l = Arc::make_mut(layer);
        for k in &mut l.keyframes {
            for e in &mut Arc::make_mut(k).elements {
                if let ElementKind::Symbol { filters, .. } = &mut Arc::make_mut(e).kind {
                    *filters = Vec::from_iter(match i {
                        0 => vec![F::DropShadow { blur_x: 8.0, blur_y: 8.0, strength: 0.8, quality: 2, angle: 45.0, distance: 10.0, color: Rgba::BLACK, inner: false, knockout: false, hide_object: false }, F::Bevel { blur_x: 5.0, blur_y: 5.0, strength: 1.0, quality: 1, angle: 45.0, distance: 4.0, shadow_color: Rgba::BLACK, highlight_color: Rgba::WHITE, side: anicraft_doc::FilterSide::Inner, knockout: false }],
                        _ => vec![F::Glow { blur_x: 12.0, blur_y: 12.0, strength: 1.4, quality: 2, color: Rgba::new(1.0, 0.85, 0.2, 1.0), inner: false, knockout: false }],
                    }.into_iter().map(anicraft_doc::FilterEntry::from));
                }
            }
        }
    }
    let shape = anicraft_doc::Shape { path: Arc::new(Path::rect(0.0, 0.0, 1280.0, 300.0)), merge: false, fill: Some(Paint::Bitmap { bitmap: anicraft_doc::BitmapId(77), repeat: true, matrix: Affine::scale(5.0, 5.0) }), stroke: None, fill_rule: FillRule::NonZero, warp: None };
    let sky = anicraft_doc::Element { id: ElementId(9_100), name: "sky".into(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(shape) };
    let frames = doc.frame_count();
    let mut k = anicraft_doc::Keyframe::new(0, frames);
    k.elements.push(Arc::new(sky));
    let mut layer = anicraft_doc::Layer::new(LayerId(9_101), "Sky");
    layer.keyframes = vec![Arc::new(k)];
    doc.timeline_mut(0).layers.push(Arc::new(layer));
    doc
}

#[test]
fn gpu_matches_the_oracle_on_the_filtered_sample() {
    let Some(g) = gpu() else { return };
    let doc = filtered_sample();
    let dl0 = anicraft_doc::evaluate(&doc, 0);
    assert!(dl0.groups.iter().any(|g| !g.filters.is_empty()) && dl0.items.iter().any(|i| i.image.is_some()), "the sample has filters and a bitmap");
    for (f, s) in [(0u32, 1.0f32), (7, 0.5), (24, 1.5), (40, 0.37)] {
        let dl = anicraft_doc::evaluate(&doc, f);
        let d = assert_parity(g, "filtered sample", &dl, &View::stage(&doc, s));
        eprintln!("filtered sample {f} at {s}: mean {:.5} / max {} / {} of {} px differ", d.mean, d.max, d.differing, d.pixels);
        // And a stage tile as the stage asks for it (512 px, zoomed and panned).
        let tile = View { transform: Affine::translate(-300.0, -200.0).mul(&Affine::scale(s * 1.4, s * 1.4)), width: 512, height: 512, background: Some(doc.stage.background) };
        assert_parity(g, "filtered sample tile", &dl, &tile);
    }
}


// ----- binning on the GPU (ADR 0048) ----------------------------------------------------------

/// Scenes the GPU bins (no fill in a mask or a group): the analytic ones, the sample, stress
/// frames at awkward scales and offsets, and onion-skin style outlines over them.
fn gpu_binned_scenes() -> Vec<(String, DisplayList, View)> {
    let plain = |dl: &DisplayList| dl.items.iter().all(|i| i.clip.is_none() && i.group.is_none());
    let mut out: Vec<(String, DisplayList, View)> = scenes().into_iter().filter(|(_, dl, _)| plain(dl)).map(|(n, dl, v)| (n.to_string(), dl, v)).collect();
    let demo = sample::demo();
    out.push(("sample".into(), anicraft_doc::evaluate(&demo, 13), View::stage(&demo, 0.37)));
    let doc = sample::stress(20, 100, 48);
    for (f, s, at) in [(0u32, 1.0f32, (0.0, 0.0)), (30, 0.5, (0.0, 0.0)), (7, 2.3, (-900.0, -700.0)), (41, 0.73, (-11.5, -3.25))] {
        let dl = anicraft_doc::evaluate(&doc, f);
        let v = View { transform: Affine::translate(at.0, at.1).mul(&Affine::scale(s, s)), width: 1920, height: 1080, background: Some(Rgba::WHITE) };
        out.push((format!("stress frame {f} at {s}"), dl, v));
    }
    // Onion skin's outlines: the frames before and after as hairlines, under the playhead's.
    let mut dl = DisplayList::default();
    for f in [25u32, 27, 33, 35] {
        let mut part = anicraft_doc::evaluate(&doc, f);
        for i in &mut part.items {
            i.outline = Some(Rgba::hex(if f < 30 { 0x4f8dff } else { 0x4cd07d }).with_alpha(0.4));
        }
        dl.append(part);
    }
    dl.append(anicraft_doc::evaluate(&doc, 30));
    out.push(("stress with outlined frames".into(), dl, View::stage(&doc, 0.8)));
    out
}

/// What an entry covers in each of its tile's 256 pixels.
fn entry_pixels(b: &Binned, e: usize) -> [f32; 256] {
    let (op, first, n, bd) = (b.index[e] as usize, b.index[e + 1] as usize, b.index[e + 2] as usize, b.index[e + 3]);
    let rule = b.op_heads[4 * op + 1];
    let pieces = b.geo.get(first..first + n).unwrap_or(&[]);
    std::array::from_fn(|i| entry_cover(pieces, bd, rule, i % 16, i / 16))
}

#[test]
fn gpu_binning_produces_the_cpus_tile_lists() {
    let Some(g) = gpu() else { return };
    for (name, dl, v) in gpu_binned_scenes() {
        let Some(gb) = g.gpu_lists(&dl, &v).unwrap() else { panic!("{name}: the GPU should bin this") };
        let (scene, _) = prepare_scene(&dl, &v).unwrap();
        let cb = binning::bin(&scene, v.width, v.height).unwrap();
        assert_eq!((gb.tiles_x, gb.tiles_y, &gb.op_heads), (cb.tiles_x, cb.tiles_y, &cb.op_heads), "{name}");
        let (mut entries, mut pieces) = (0usize, 0usize);
        for t in 0..(cb.tiles_x * cb.tiles_y) as usize {
            let list = |b: &Binned| -> Vec<(u32, [f32; 256])> {
                let (s, n) = (b.index[2 * t] as usize, b.index[2 * t + 1] as usize);
                (s..s + n).map(|i| b.entries_at as usize + 4 * i).map(|e| (b.index[e], entry_pixels(b, e))).collect()
            };
            let (a, c) = (list(&gb), list(&cb));
            let ops = |l: &[(u32, [f32; 256])]| l.iter().map(|x| x.0).collect::<Vec<_>>();
            assert_eq!(ops(&a), ops(&c), "{name}: tile {t}'s fills");
            for ((op, pa), (_, pc)) in a.iter().zip(&c) {
                let worst = pa.iter().zip(pc).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
                assert!(worst < 1e-3, "{name}: tile {t}, op {op}: coverage off by {worst}");
            }
            entries += a.len();
            let (s, n) = (gb.index[2 * t] as usize, gb.index[2 * t + 1] as usize);
            pieces += (s..s + n).map(|i| gb.index[gb.entries_at as usize + 4 * i + 2] as usize).sum::<usize>();
        }
        assert_eq!(entries, cb.entries, "{name}");
        assert!(pieces <= gb.pieces && entries <= gb.entries, "{name}: within the room made for them");
        // And the fine pass's arithmetic over the GPU's lists draws the oracle's picture.
        let (oracle, _) = render(&dl, &v).unwrap();
        let eg = emulate(&gb, &v);
        let (mean, max) = f32_diff(&eg, &oracle.data);
        assert!(mean < 1e-4 && max < 4e-3, "{name}: mean {mean:e}, max {max:e}");
    }
}

#[test]
fn gpu_binned_frames_match_the_oracle() {
    let Some(g) = gpu() else { return };
    // Where the GPU stage runs (lavapipe in CI included), its binning must too.
    assert_eq!(g.gpu_binning_off(), None);
    for (name, dl, v) in gpu_binned_scenes() {
        let f = g.render_with(&dl, &v, Output::Premul8, Binning::Gpu).unwrap();
        assert!(f.gpu_binned, "{name}");
        let (oracle, _) = render(&dl, &v).unwrap();
        let d = compare_premul8(&f.premul8, &oracle.to_premul_rgba8()).unwrap();
        assert!(d.mean <= MEAN_TARGET / 16.0 && d.max <= 2, "{name}: {d:?}");
        let t = g.render_texture_with(&dl, &v, Binning::Gpu).unwrap();
        assert!(t.gpu_binned && g.read_texture(&t).unwrap() == f.premul8, "{name}: the texture holds the read-back bytes");
    }
    // Masks and groups stay on the CPU's binning, and still draw right.
    for (name, dl, v) in composited_scenes() {
        let f = g.render_with(&dl, &v, Output::Premul8, Binning::Gpu).unwrap();
        let uses = |dl: &DisplayList| dl.items.iter().any(|i| i.clip.is_some() || i.group.is_some());
        assert_eq!(f.gpu_binned, !uses(&dl), "{name}: binned on the GPU {}", f.gpu_binned);
        let (oracle, _) = render(&dl, &v).unwrap();
        assert!(compare_premul8(&f.premul8, &oracle.to_premul_rgba8()).unwrap().max <= 2, "{name}");
    }
}
