//! Clip paths and masks → mask layers (ADR 0021), on self-authored drawings.

use super::*;
use anicraft_doc::{MaskMode, Paint, Rgba};

fn imp(svg: &str) -> SvgImport {
    match import_with_fonts(svg.as_bytes(), &Fonts::from_faces(Vec::new())) {
        Ok(i) => i,
        Err(e) => panic!("{e}: {svg}"),
    }
}

fn names(l: &ImportedLayer) -> Vec<&str> {
    l.elements.iter().map(|e| e.name.as_str()).collect()
}

fn path_of(e: &ImportedElement) -> Path {
    match &e.kind {
        ElementKind::Shape(s) => s.path.transformed(&e.transform),
        other => panic!("not a shape: {other:?}"),
    }
}

/// Whether drawing-space point `p` is inside the mask layer's fills.
fn in_mask(m: &ImportedMask, p: Vec2) -> bool {
    m.elements.iter().any(|e| match &e.kind {
        ElementKind::Shape(s) => s.path.contains(p, &e.transform, s.fill_rule),
        _ => false,
    })
}

#[test]
fn a_clipped_group_becomes_a_masked_layer_under_its_mask() {
    let i = imp(r##"<svg width="200" height="100">
      <defs><clipPath id="round"><circle cx="50" cy="50" r="40"/></clipPath></defs>
      <rect id="bg" width="200" height="100" fill="#eee"/>
      <g clip-path="url(#round)"><rect id="a" width="100" height="100" fill="red" stroke="#000"/><rect id="b" x="20" y="20" width="30" height="30" fill="blue"/></g>
      <rect id="top" x="150" width="10" height="10"/>
    </svg>"##);
    assert_eq!(i.layers.len(), 3, "{:?}", i.layers);
    assert!(i.layers[0].mask.is_none() && i.layers[2].mask.is_none());
    assert_eq!((names(&i.layers[0]), names(&i.layers[1]), names(&i.layers[2])), (vec!["bg"], vec!["a", "b"], vec!["top"]));
    let mask = i.layers[1].mask.as_ref().expect("masked");
    assert_eq!(mask.name, "round");
    assert_eq!(mask.elements.len(), 1);
    assert!(in_mask(mask, Vec2::new(50.0, 50.0)) && !in_mask(mask, Vec2::new(5.0, 5.0)));
    let b = path_of(&mask.elements[0]).bounds();
    assert!((b.min.x - 10.0).abs() < 1e-3 && (b.max.x - 90.0).abs() < 1e-3, "{b:?}");
    // The artwork keeps its paint and stroke; the mask's own artwork is a plain fill.
    assert!(matches!(&i.layers[1].elements[0].kind, ElementKind::Shape(s) if s.stroke.is_some()));
    assert!(i.report.skipped.is_empty(), "{:?}", i.report);
    assert_eq!(i.elements().count(), 4, "mask artwork isn't drawn");
}

#[test]
fn clips_that_cut_nothing_go_and_clips_that_hide_everything_drop_the_artwork() {
    // A frame clip around the whole drawing (as design tools export) changes nothing.
    let i = imp(r##"<svg width="100" height="100">
      <clipPath id="frame"><rect width="100" height="100"/></clipPath>
      <clipPath id="far"><rect x="500" y="500" width="10" height="10"/></clipPath>
      <clipPath id="empty"/>
      <g clip-path="url(#frame)"><rect id="a" x="10" y="10" width="20" height="20" stroke="#000" stroke-width="2"/></g>
      <rect id="gone" width="10" height="10" clip-path="url(#far)"/>
      <rect id="gone too" width="10" height="10" clip-path="url(#empty)"/>
      <rect id="kept" width="10" height="10" clip-path="url(#nowhere)"/>
    </svg>"##);
    assert_eq!(i.layers.len(), 1, "{:?}", i.layers);
    assert!(i.layers[0].mask.is_none());
    assert_eq!(names(&i.layers[0]), ["a", "kept"]);
    assert_eq!(i.report.skipped.get("references to missing clip paths (drawn unclipped)"), Some(&1));
}

#[test]
fn clips_inside_clips_intersect_exactly() {
    // A circle clip inside a band clip: the mask is the circle cut to the band, still curved.
    let i = imp(r##"<svg width="200" height="200">
      <clipPath id="band"><rect x="0" y="40" width="200" height="40"/></clipPath>
      <clipPath id="disc"><circle cx="60" cy="60" r="40"/></clipPath>
      <g clip-path="url(#band)"><rect id="wide" width="200" height="200" fill="red"/><g clip-path="url(#disc)"><rect id="inner" width="200" height="200" fill="blue"/></g></g>
    </svg>"##);
    assert_eq!(i.layers.len(), 2, "{:?}", i.layers);
    let (outer, inner) = (i.layers[0].mask.as_ref().unwrap(), i.layers[1].mask.as_ref().unwrap());
    assert_eq!((outer.name.as_str(), inner.name.as_str()), ("band", "disc"));
    let p = path_of(&inner.elements[0]);
    assert!(p.cmds.iter().any(|c| matches!(c, anicraft_geom::PathCmd::CubicTo(..))), "the arcs stay curves");
    let b = p.bounds();
    assert!(b.min.y >= 40.0 - 1e-3 && b.max.y <= 80.0 + 1e-3 && b.min.x >= 20.0 - 1e-3 && b.max.x <= 100.0 + 1e-3, "{b:?}");
    assert!(in_mask(inner, Vec2::new(60.0, 60.0)) && !in_mask(inner, Vec2::new(60.0, 30.0)) && !in_mask(inner, Vec2::new(150.0, 60.0)));
    // Two curved clips can't be intersected exactly: one is kept, and the report says so.
    let two = imp(r##"<svg width="200" height="200">
      <clipPath id="a"><circle cx="60" cy="60" r="40"/></clipPath>
      <clipPath id="b"><circle cx="90" cy="60" r="40"/></clipPath>
      <g clip-path="url(#a)"><rect clip-path="url(#b)" width="200" height="200"/></g>
    </svg>"##);
    assert_eq!(two.layers.len(), 1);
    assert_eq!(two.report.skipped.get("curved clips inside curved clips (drawn with the inner one)"), Some(&1));
}

#[test]
fn bounding_box_units_and_transforms_place_the_clip() {
    let i = imp(r##"<svg width="200" height="200">
      <clipPath id="oval" clipPathUnits="objectBoundingBox"><circle cx=".5" cy=".5" r=".5"/></clipPath>
      <clipPath id="user"><rect x="0" y="0" width="10" height="10" transform="rotate(45)"/></clipPath>
      <rect id="a" x="10" y="10" width="100" height="50" clip-path="url(#oval)"/>
      <g transform="translate(150 150)"><rect id="b" x="-20" y="-20" width="40" height="40" clip-path="url(#user)"/></g>
    </svg>"##);
    assert_eq!(i.layers.len(), 2);
    let oval = path_of(&i.layers[0].mask.as_ref().unwrap().elements[0]).bounds();
    assert!((oval.min.x - 10.0).abs() < 1e-3 && (oval.max.x - 110.0).abs() < 1e-3 && (oval.min.y - 10.0).abs() < 1e-3 && (oval.max.y - 60.0).abs() < 1e-3, "{oval:?}");
    // A userSpaceOnUse clip lives in the space of the element it clips (inside its group).
    let user = i.layers[1].mask.as_ref().unwrap();
    assert!(in_mask(user, Vec2::new(150.0, 155.0)) && !in_mask(user, Vec2::new(155.0, 150.0)), "rotated 45° about the group's origin");
}

#[test]
fn shapes_sharing_one_clip_share_one_mask_layer() {
    // What SVG export writes: every masked path names the same clip.
    let i = imp(r##"<svg width="100" height="100">
      <defs><clipPath id="c0" clipPathUnits="userSpaceOnUse"><path d="M50 10 L90 90 L10 90 Z"/></clipPath></defs>
      <path d="M0 0 H100 V100 H0 Z" fill="#00f" clip-path="url(#c0)"/>
      <path d="M0 40 H100 V60 H0 Z" fill="#f00" clip-path="url(#c0)"/>
      <path d="M0 0 H10 V10 H0 Z" fill="#0f0"/>
      <path d="M0 0 H100 V100 H0 Z" fill="#00f" clip-path="url(#c0)" transform="translate(1 0)"/>
    </svg>"##);
    let shape: Vec<(bool, usize)> = i.layers.iter().map(|l| (l.mask.is_some(), l.elements.len())).collect();
    assert_eq!(shape, [(true, 2), (false, 1), (true, 1)], "a clip in another space is another mask");
}

#[test]
fn opaque_masks_become_mask_layers_and_soft_ones_soft_mask_layers() {
    let i = imp(r##"<svg width="100" height="100">
      <mask id="white"><circle cx="50" cy="50" r="30" fill="white"/><rect x="0" y="0" width="10" height="10" style="fill:#fff"/></mask>
      <mask id="alpha" style="mask-type:alpha"><circle cx="50" cy="50" r="30" fill="#123456"/></mask>
      <mask id="grey"><circle cx="50" cy="50" r="30" fill="#808080"/></mask>
      <mask id="faded"><circle cx="50" cy="50" r="30" fill="white" opacity="0.5"/></mask>
      <mask id="tight" x="0.25" y="0.25" width="0.5" height="0.5"><rect width="100" height="100" fill="white"/></mask>
      <rect id="a" width="100" height="100" mask="url(#white)"/>
      <rect id="b" width="100" height="100" mask="url(#alpha)"/>
      <rect id="c" width="100" height="100" mask="url(#grey)"/>
      <rect id="d" width="100" height="100" mask="url(#faded)"/>
      <rect id="e" width="100" height="100" mask="url(#tight)"/>
    </svg>"##);
    let shape: Vec<(Option<&str>, Vec<&str>)> = i.layers.iter().map(|l| (l.mask.as_ref().map(|m| m.name.as_str()), names(l))).collect();
    assert_eq!(shape, [(Some("white"), vec!["a"]), (Some("alpha"), vec!["b"]), (Some("grey"), vec!["c"]), (Some("faded"), vec!["d"]), (Some("tight"), vec!["e"])]);
    let modes: Vec<MaskMode> = i.layers.iter().map(|l| l.mask.as_ref().unwrap().mode).collect();
    // Opaque masks stay Animate's masks (by area); the grey and the faded one are soft
    // luminance masks (ADR 0070), their artwork as drawn.
    assert_eq!(modes, [MaskMode::Coverage, MaskMode::Coverage, MaskMode::Luminance, MaskMode::Luminance, MaskMode::Coverage]);
    let white = i.layers[0].mask.as_ref().unwrap();
    assert_eq!(white.elements.len(), 2, "the default mask region (bounding box + 10%) holds the content and cuts nothing");
    assert!(in_mask(white, Vec2::new(5.0, 5.0)) && in_mask(white, Vec2::new(50.0, 50.0)) && !in_mask(white, Vec2::new(90.0, 10.0)));
    let grey = &i.layers[2].mask.as_ref().unwrap().elements[0];
    assert!(matches!(&grey.kind, ElementKind::Shape(s) if s.fill == Some(Paint::Solid(Rgba::hex(0x808080)))), "{grey:?}");
    assert_eq!(i.layers[3].mask.as_ref().unwrap().elements[0].alpha, 0.5, "the content's opacity stays");
    assert!(i.report.skipped.is_empty(), "{:?}", i.report);
    // A mask region cuts the mask: the full-size rect becomes the middle quarter.
    let tight = path_of(&i.layers[4].mask.as_ref().unwrap().elements[0]).bounds();
    assert_eq!(tight, Rect::new(25.0, 25.0, 75.0, 75.0));
}

#[test]
fn soft_masks_keep_gradients_and_their_region_cuts_them() {
    let i = imp(r##"<svg width="200" height="100">
      <linearGradient id="fade"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>
      <linearGradient id="dark"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></linearGradient>
      <mask id="a" mask-type="alpha" maskUnits="userSpaceOnUse" x="0" y="0" width="50" height="100"><rect width="200" height="100" fill="url(#fade)"/></mask>
      <mask id="l"><rect width="200" height="100" fill="url(#dark)" stroke="#fff"/></mask>
      <rect id="cut" width="200" height="100" mask="url(#a)"/>
      <rect id="lum" width="200" height="100" fill="red" mask="url(#l)"/>
    </svg>"##);
    assert_eq!(i.layers.len(), 2, "{:?}", i.layers);
    let a = i.layers[0].mask.as_ref().unwrap();
    assert_eq!(a.mode, MaskMode::Alpha);
    // The region (x 0..50) cuts the gradient-filled rect in its own space; the paint stays.
    let ElementKind::Shape(s) = &a.elements[0].kind else { panic!() };
    assert!(matches!(s.fill, Some(Paint::Linear { .. })));
    let b = path_of(&a.elements[0]).bounds();
    assert!((b.max.x - 50.0).abs() < 1e-3 && b.min.x.abs() < 1e-3, "{b:?}");
    // Luminance by default; the stroke stays too.
    let l = i.layers[1].mask.as_ref().unwrap();
    assert_eq!(l.mode, MaskMode::Luminance);
    assert!(matches!(&l.elements[0].kind, ElementKind::Shape(s) if s.stroke.is_some() && matches!(s.fill, Some(Paint::Linear { .. }))));
}

#[test]
fn loops_and_floods_of_clips_stay_bounded() {
    let start = std::time::Instant::now();
    let mut svg = String::from(r##"<svg width="100" height="100">
      <clipPath id="self" clip-path="url(#self)"><rect width="50" height="50"/></clipPath>
      <mask id="m"><rect width="100" height="100" fill="white" mask="url(#m)"/><use href="#user"/></mask>
      <rect id="user" width="60" height="60" mask="url(#m)"/>
      <rect width="80" height="80" clip-path="url(#self)"/>"##);
    for k in 0..300 {
        svg.push_str(&format!(r##"<clipPath id="c{k}"><circle cx="{}" cy="50" r="5"/></clipPath><rect width="100" height="100" clip-path="url(#c{k})"/>"##, 5 + k % 90));
    }
    svg.push_str("</svg>");
    let i = imp(&svg);
    assert!(start.elapsed().as_secs() < 10, "{:?}", start.elapsed());
    assert!(i.report.skipped.contains_key("clip paths or masks that refer to themselves (ignored)"), "{:?}", i.report);
    assert!(i.layers.iter().filter(|l| l.mask.is_some()).count() <= clip::MAX_MASKS);
    assert!(i.report.skipped.contains_key("clip paths and masks past 250 per drawing (drawn unclipped)"));
}

#[test]
fn the_fixture_drawing_comes_across_as_five_layers() {
    let i = imp(include_str!("../fixtures/clips-masks-text.svg"));
    let shape: Vec<Option<&str>> = i.layers.iter().map(|l| l.mask.as_ref().map(|m| m.name.as_str())).collect();
    // The frame clip holds everything and goes; the sun and sea sit in the porthole, the red
    // ball in the porthole cut to the band, the stripes under the mask.
    assert_eq!(shape[..4], [None, Some("porthole"), Some("porthole"), Some("reveal")], "{shape:?}");
    assert_eq!(i.layers[1].elements.len(), 3);
    assert_eq!(i.layers[3].elements.len(), 3);
    let ball = path_of(&i.layers[2].mask.as_ref().unwrap().elements[0]).bounds();
    assert!(ball.min.y >= 95.0 - 1e-3 && ball.max.y <= 155.0 + 1e-3, "{ball:?}");
    assert_eq!(i.report.skipped.get("text (no fonts on this computer)"), Some(&2), "{:?}", i.report);
}
