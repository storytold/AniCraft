use super::*;

/// An imported shape, flattened out of its layer.
#[derive(Clone, Debug)]
struct ImportedShape {
    name: String,
    transform: Affine,
    alpha: f32,
    shape: Shape,
}

/// An import whose layers are flattened into its shapes.
#[derive(Clone, Debug)]
struct Flat {
    width: f32,
    height: f32,
    shapes: Vec<ImportedShape>,
    report: Report,
}

fn imp(svg: &str) -> Flat {
    let i = match import_with_fonts(svg.as_bytes(), &Fonts::from_faces(Vec::new())) {
        Ok(i) => i,
        Err(e) => panic!("{e}: {svg}"),
    };
    let shapes = i
        .elements()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(s) => Some(ImportedShape { name: e.name.clone(), transform: e.transform, alpha: e.alpha, shape: s.clone() }),
            _ => None,
        })
        .collect();
    Flat { width: i.width, height: i.height, shapes, report: i.report }
}

fn solid(s: &ImportedShape) -> Rgba {
    match &s.shape.fill {
        Some(Paint::Solid(c)) => *c,
        other => panic!("not solid: {other:?}"),
    }
}

#[test]
fn shapes_styles_and_inheritance() {
    let i = imp(r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
      <g fill="#ff0000" stroke="blue" stroke-width="4" opacity="0.5">
        <rect id="box" x="10" y="10" width="50" height="30"/>
        <circle cx="100" cy="50" r="20" style="fill: rgb(0, 128, 255); stroke:none"/>
        <ellipse cx="150" cy="50" rx="30" ry="10" fill="none"/>
        <line x1="0" y1="0" x2="200" y2="100"/>
        <polygon points="0,0 10,0 10,10" fill-rule="evenodd" fill-opacity="50%"/>
      </g>
      <path d="M0 0 L10 10" fill="currentColor" color="#00ff00" display="none"/>
    </svg>"##);
    assert_eq!((i.width, i.height), (200.0, 100.0));
    assert_eq!(i.shapes.len(), 5, "display:none is skipped");
    let r = &i.shapes[0];
    assert_eq!(r.name, "box");
    assert_eq!(solid(r), Rgba::hex(0xff0000));
    // SVG's initial stroke style: butt caps, miter joins, a miter limit of 4.
    let svg_style = StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0, dash: None };
    assert_eq!(r.shape.stroke, Some(Stroke::new(Rgba::hex(0x0000ff), 4.0).with_style(svg_style)));
    assert_eq!(r.alpha, 0.5, "group opacity reaches every child");
    assert_eq!(r.shape.path.bounds(), Rect::new(10.0, 10.0, 60.0, 40.0));
    let c = &i.shapes[1];
    assert!((solid(c).g - 128.0 / 255.0).abs() < 1e-4, "style= wins over the inherited fill");
    assert_eq!(c.shape.stroke, None);
    assert!(i.shapes[2].shape.fill.is_none() && i.shapes[2].shape.stroke.is_some(), "an unfilled ellipse keeps its stroke");
    assert!(i.shapes[3].shape.fill.is_none(), "lines are never filled");
    let p = &i.shapes[4];
    assert_eq!(p.shape.fill_rule, FillRule::EvenOdd);
    assert!((solid(p).a - 0.5).abs() < 1e-6);
    assert!(i.report.skipped.is_empty(), "{:?}", i.report);
}

#[test]
fn transforms_view_box_and_units() {
    // A 2 × 1 inch drawing whose viewBox is 0..20 × 0..10: one unit = 9.6 px.
    let i = imp(r##"<svg width="2in" height="1in" viewBox="0 0 20 10">
      <g transform="translate(5 0) scale(2)"><rect width="1" height="1" transform="rotate(90)"/></g>
    </svg>"##);
    assert_eq!((i.width, i.height), (192.0, 96.0));
    let m = i.shapes[0].transform;
    let p = m.apply(Vec2::new(1.0, 0.0));
    // (1,0) → rotate 90° → (0,1) → scale 2 → (0,2) → translate → (5,2) → ×9.6
    assert!((p.x - 48.0).abs() < 1e-3 && (p.y - 19.2).abs() < 1e-3, "{p:?}");
    // meet centres a mismatched viewBox; none stretches it.
    let meet = imp(r#"<svg width="100" height="50" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"#);
    let q = meet.shapes[0].transform.apply(Vec2::new(0.0, 0.0));
    assert!((q.x - 25.0).abs() < 1e-4 && q.y.abs() < 1e-4, "{q:?}");
    let none = imp(r#"<svg width="100" height="50" viewBox="0 0 10 10" preserveAspectRatio="none"><rect width="10" height="10"/></svg>"#);
    assert_eq!(none.shapes[0].transform.apply(Vec2::new(10.0, 10.0)), Vec2::new(100.0, 50.0));
    assert_eq!(transform("matrix(1 0 0 1 3 4) skewX(45)").apply(Vec2::new(0.0, 1.0)), Vec2::new(4.0, 5.0));
    assert_eq!(length("72pt", 0.0), Some(96.0));
    assert_eq!(length("50%", 300.0), Some(150.0));
    assert_eq!(length("3furlongs", 1.0), None);
}

#[test]
fn gradients_in_both_unit_systems_with_inherited_stops() {
    let i = imp(r##"<svg width="100" height="100" xmlns:xlink="http://www.w3.org/1999/xlink">
      <defs>
        <linearGradient id="base"><stop offset="0" stop-color="#000"/><stop offset="100%" style="stop-color:#fff;stop-opacity:0.5"/></linearGradient>
        <linearGradient id="h" xlink:href="#base" x1="0" y1="0" x2="1" y2="0"/>
        <radialGradient id="r" gradientUnits="userSpaceOnUse" cx="50" cy="50" r="25" href="#base"/>
        <linearGradient id="diag" href="#base" x2="1" y2="1"/>
      </defs>
      <rect x="0" y="0" width="90" height="10" fill="url(#diag)"/>
      <rect x="20" y="10" width="40" height="80" fill="url(#h)"/>
      <circle cx="50" cy="50" r="25" fill="url(#r)" fill-opacity="0.5"/>
      <rect width="5" height="5" fill="url(#nothing) red"/>
    </svg>"##);
    let i = Flat { shapes: i.shapes[1..].to_vec(), ..i };
    match &i.shapes[0].shape.fill {
        Some(Paint::Linear { start, end, stops, .. }) => {
            assert_eq!((*start, *end), (Vec2::new(20.0, 10.0), Vec2::new(60.0, 10.0)), "bounding-box units map onto the shape");
            assert_eq!(stops.len(), 2);
            assert_eq!(stops[1].color, Rgba::WHITE.with_alpha(0.5));
        }
        other => panic!("{other:?}"),
    }
    match &i.shapes[1].shape.fill {
        Some(Paint::Radial { center, radius, stops, .. }) => {
            assert_eq!((*center, *radius), (Vec2::new(50.0, 50.0), 25.0));
            assert_eq!(stops[0].color.a, 0.5, "fill-opacity folds into the stops");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(solid(&i.shapes[2]), Rgba::hex(0xff0000), "a missing paint server uses the fallback colour");
    // Only the diagonal gradient in the wide strip tilts (approximated): the tall box's
    // horizontal gradient stretches along its axis, which is exact. #nothing is unknown.
    assert_eq!(i.report.skipped.get("stretched or skewed gradients (approximated)"), Some(&1));
    assert_eq!(i.report.skipped.get("pattern or unknown fills"), Some(&1));
}

#[test]
fn use_references_and_reported_leftovers() {
    let i = imp(r##"<svg width="100" height="100">
      <defs><circle id="dot" r="5" fill="#123456"/><symbol id="pair"><rect width="2" height="2"/><rect x="4" width="2" height="2"/></symbol></defs>
      <use href="#dot" x="10" y="20"/>
      <use xlink:href="#pair" x="50" y="50"/>
      <use href="#missing"/>
      <text x="0" y="10">Hello</text>
      <image href="photo.png" width="10" height="10"/>
      <rect width="10" height="10" stroke="#000" stroke-dasharray="2 2" filter="url(#f)" clip-path="url(#c)"/>
      <style>rect { fill: red }</style>
    </svg>"##);
    let first = &i.shapes[0];
    assert_eq!(first.transform.apply(Vec2::new(0.0, 0.0)), Vec2::new(10.0, 20.0));
    assert_eq!(solid(first), Rgba::hex(0x123456));
    assert_eq!(i.shapes.len(), 4, "the dot, the symbol's two rects and the plain rect");
    for k in ["text (no fonts on this computer)", "images", "references to missing elements", "filters", "references to missing clip paths (drawn unclipped)", "style sheets (inline styles are read)"] {
        assert_eq!(i.report.skipped.get(k), Some(&1), "{k}: {:?}", i.report);
    }
    assert!(i.report.summary().starts_with("Imported. Not yet supported: "));
}

#[test]
fn stroke_caps_joins_and_dashes() {
    let i = imp(r##"<svg width="200" height="100">
      <g stroke="#000" stroke-linecap="round" stroke-linejoin="bevel" stroke-dasharray="6, 3" stroke-dashoffset="2">
        <line id="inherits" x1="0" y1="0" x2="100" y2="0"/>
        <line id="own" x1="0" y1="0" x2="100" y2="0" style="stroke-linecap: square; stroke-linejoin: miter-clip; stroke-miterlimit: 8; stroke-dasharray: 1pt 2 3"/>
        <line id="none" x1="0" y1="0" x2="100" y2="0" stroke-dasharray="none" stroke-linejoin="arcs"/>
        <line id="negative" x1="0" y1="0" x2="100" y2="0" stroke-dasharray="4 -1"/>
        <line id="junk" x1="0" y1="0" x2="100" y2="0" stroke-dasharray="4 wide" stroke-miterlimit="0.5" stroke-linecap="fancy"/>
        <line id="no-gaps" x1="0" y1="0" x2="100" y2="0" stroke-dasharray="0 0"/>
      </g>
      <line id="plain" x1="0" y1="0" x2="100" y2="0" stroke="#000"/>
    </svg>"##);
    let st = |name: &str| i.shapes.iter().find(|s| s.name == name).and_then(|s| s.shape.stroke.clone()).map(|s| s.style).unwrap();
    assert_eq!(st("inherits"), StrokeStyle { cap: Cap::Round, join: Join::Bevel, miter_limit: 4.0, dash: Some(Dash::Dashed { lengths: vec![6.0, 3.0].into(), offset: 2.0 }) });
    // style= wins; an odd list repeats; units convert; the offset is inherited.
    let own = st("own");
    assert_eq!((own.cap, own.join, own.miter_limit), (Cap::Square, Join::Miter, 8.0));
    let pt = 96.0 / 72.0;
    assert_eq!(own.dash, Some(Dash::Dashed { lengths: vec![pt, 2.0, 3.0, pt, 2.0, 3.0].into(), offset: 2.0 }));
    assert_eq!((st("none").dash, st("none").join), (None, Join::Round));
    // Invalid arrays draw solid (as SVG renders them); unknown values keep the inherited ones.
    assert_eq!(st("negative").dash, None);
    let junk = st("junk");
    assert_eq!((junk.dash, junk.miter_limit, junk.cap), (None, 4.0, Cap::Round));
    assert_eq!(st("no-gaps").dash, None);
    assert_eq!(st("plain"), StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0, dash: None });
    assert!(i.report.skipped.is_empty(), "{:?}", i.report);
}

#[test]
fn dash_lists() {
    assert_eq!(dash_array("5,3 2"), Some(vec![5.0, 3.0, 2.0]));
    assert_eq!(dash_array(" 10% , 1in "), Some(vec![10.0, 96.0]));
    assert_eq!(dash_array("none"), None);
    assert_eq!(dash_array(""), None);
    assert_eq!(dash_array("1 -2"), None);
    assert_eq!(dash_array("1 nan"), None);
}

#[test]
fn colours() {
    assert_eq!(color("#f00"), Some(Rgba::hex(0xff0000)));
    assert_eq!(color("#ff000080").map(|c| (c.a * 255.0).round()), Some(128.0));
    assert_eq!(color("rgba(255, 0, 0, 0.25)"), Some(Rgba::new(1.0, 0.0, 0.0, 0.25)));
    assert_eq!(color("rgb(100%, 50%, 0%)"), Some(Rgba::new(1.0, 0.5, 0.0, 1.0)));
    assert_eq!(color("SteelBlue"), Some(Rgba::hex(0x4682b4)));
    assert_eq!(color("transparent"), Some(Rgba::TRANSPARENT));
    for bad in ["", "#12", "rgb(1,2)", "notacolour", "#ééé"] {
        assert_eq!(color(bad), None, "{bad}");
    }
}

#[test]
fn hostile_input_is_an_error_or_harmless() {
    assert!(matches!(import(b"<html/>"), Err(SvgError::NotSvg(_))));
    assert!(import(b"<svg").is_err());
    // Self-referencing uses, a reference loop between gradients, NaN-producing transforms
    // and absurd sizes: all finish quickly with nothing broken.
    let i = imp(r##"<svg width="-5" height="1e40">
      <g id="loop"><use href="#loop"/><rect width="1" height="1"/></g>
      <linearGradient id="a" href="#b"/><linearGradient id="b" href="#a"/>
      <rect width="1" height="1" fill="url(#a)" transform="scale(1e30) scale(1e30)"/>
      <rect width="1" height="1" transform="matrix(1 2)"/>
      <path d="M 0 0 L"/>
    </svg>"##);
    assert_eq!(i.width, 300.0, "a negative width falls back to the default");
    assert!(i.shapes.iter().all(|s| s.transform.is_finite()));
    assert!(i.shapes.len() < 40, "{}", i.shapes.len());
    assert!(i.report.skipped.contains_key("references nested too deeply"));
    assert!(i.report.skipped.contains_key("unreadable paths"));
}
