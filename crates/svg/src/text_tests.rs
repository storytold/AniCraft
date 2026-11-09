//! SVG text → text blocks (ADR 0021), laid out with this computer's fonts.

use super::*;
use anicraft_doc::{TextAlign, TextBlock};

/// This computer's fonts, or `None` (and the test passes) on one without fonts.
fn fonts() -> Option<&'static Fonts> {
    let f = Fonts::get();
    f.resolve("").map(|_| f)
}

fn texts(svg: &str, fonts: &Fonts) -> (Vec<(ImportedElement, TextBlock)>, Report) {
    let i = match import_with_fonts(svg.as_bytes(), fonts) {
        Ok(i) => i,
        Err(e) => panic!("{e}"),
    };
    let t = i
        .elements()
        .filter_map(|e| match &e.kind {
            ElementKind::Text(t) => Some((e.clone(), (**t).clone())),
            _ => None,
        })
        .collect();
    (t, i.report)
}

/// Drawing-space bounds of a block's glyphs.
fn ink(e: &ImportedElement, t: &TextBlock) -> Rect {
    t.outline.bounds().transformed(&e.transform)
}

#[test]
fn text_sits_on_its_baseline_in_its_style() {
    let Some(fonts) = fonts() else { return };
    let (t, report) = texts(r##"<svg width="300" height="100"><text id="title" x="10" y="50" font-family="'Zz No Such Face', sans-serif" font-size="24" font-weight="700" fill="#f00" fill-opacity="0.5" letter-spacing="2">Hello</text></svg>"##, fonts);
    assert_eq!(t.len(), 1);
    let (e, b) = &t[0];
    assert_eq!((e.name.as_str(), b.text.as_str(), b.size, b.bold, b.italic, b.letter_spacing), ("title", "Hello", 24.0, true, false, 2.0));
    assert_eq!(b.color, Rgba::hex(0xff0000).with_alpha(0.5));
    assert_eq!(b.font, fonts.find_face("_sans").family.unwrap(), "the first installed family of the list");
    let r = ink(e, b);
    assert!((r.max.y - 50.0).abs() < 1.5, "letters stand on the baseline: {r:?}");
    assert!(r.min.x >= 9.0 && r.min.x < 14.0, "{r:?}");
    assert!(report.missing_fonts.is_empty(), "a later family in the list is installed: {report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
}

#[test]
fn anchors_chunks_and_whitespace() {
    let Some(fonts) = fonts() else { return };
    let (t, report) = texts(
        r##"<svg width="400" height="200" font-family="sans-serif">
          <text x="200" y="40" text-anchor="middle" font-size="20">  Centred
             title  </text>
          <text x="390" y="80" text-anchor="end" font-size="20">Right</text>
          <text transform="translate(10 100)" font-size="16"><tspan x="0" y="0">Line one</tspan><tspan x="0" y="24" font-style="italic">Line two</tspan></text>
          <text x="10" y="180" font-size="16" dx="5">Mostly plain <tspan font-weight="bold">bold</tspan> words</text>
        </svg>"##,
        fonts,
    );
    let words: Vec<&str> = t.iter().map(|(_, b)| b.text.as_str()).collect();
    assert_eq!(words, ["Centred title ", "Right", "Line one", "Line two", "Mostly plain bold words"]);
    let (e, b) = &t[0];
    let c = ink(e, b).center().x;
    assert!((c - 200.0).abs() < 2.0, "middle anchor centres the line: {c}");
    assert_eq!(b.align, TextAlign::Center);
    let r = ink(&t[1].0, &t[1].1);
    assert!((r.max.x - 390.0).abs() < 2.0 && t[1].1.align == TextAlign::Right, "end anchor: {r:?}");
    // Each positioned tspan is its own block, placed through the text's transform.
    let (l1, l2) = (ink(&t[2].0, &t[2].1), ink(&t[3].0, &t[3].1));
    assert!((l1.max.y - 100.0).abs() < 5.0 && (l2.max.y - 124.0).abs() < 5.0, "{l1:?} {l2:?}");
    assert!(t[3].1.italic && !t[2].1.italic);
    // A leading dx moves the chunk; the bold word is drawn in the main style and counted.
    assert!(ink(&t[4].0, &t[4].1).min.x >= 14.0);
    assert!(!t[4].1.bold);
    assert_eq!(report.skipped.get("text with mixed styles (drawn in its main style)"), Some(&1), "{report:?}");
}

#[test]
fn missing_fonts_and_unsupported_text_are_reported() {
    let Some(fonts) = fonts() else { return };
    let (t, report) = texts(
        r##"<svg width="400" height="200">
          <text x="0" y="20" font-family="Zz Missing Face" font-size="12">One</text>
          <text x="0" y="40" style="font-family: 'Zz Missing Face'; text-decoration: underline">Two</text>
          <text x="0 10 20" y="60" stroke="#000">Three</text>
          <text x="0" y="80"><textPath href="#p">Four</textPath></text>
          <text x="0" y="100" fill="none" stroke="red">Outline</text>
          <text x="0" y="120"> </text>
          <text x="0" y="140" font-size="0">Tiny</text>
        </svg>"##,
        fonts,
    );
    assert_eq!(t.len(), 3, "{:?}", t.iter().map(|(_, b)| &b.text).collect::<Vec<_>>());
    assert_eq!(report.missing_fonts.iter().map(String::as_str).collect::<Vec<_>>(), ["Zz Missing Face"]);
    assert!(fonts.families().contains(report.fallback_font.as_ref().unwrap()));
    assert!(report.summary().contains("Fonts not on this computer, drawn in"), "{}", report.summary());
    for k in ["underlined or struck-through text (lines left out)", "per-character text positions (ignored)", "strokes on text (left out)", "text on a path", "outlined text without a fill (left out)"] {
        assert_eq!(report.skipped.get(k), Some(&1), "{k}: {report:?}");
    }
}

#[test]
fn text_can_be_a_clip_and_needs_fonts() {
    let none = Fonts::from_faces(Vec::new());
    let (t, report) = texts(r#"<svg width="10" height="10"><text>Hi</text></svg>"#, &none);
    assert!(t.is_empty());
    assert_eq!(report.skipped.get("text (no fonts on this computer)"), Some(&1));
    let Some(fonts) = fonts() else { return };
    let i = import_with_fonts(
        br##"<svg width="300" height="100"><clipPath id="word"><text x="10" y="60" font-size="48" font-family="sans-serif">REVEAL</text></clipPath><rect width="300" height="100" fill="#4fa3ff" clip-path="url(#word)"/></svg>"##,
        fonts,
    )
    .unwrap();
    let mask = i.layers.iter().find_map(|l| l.mask.as_ref()).expect("a mask layer");
    assert!(matches!(&mask.elements[0].kind, ElementKind::Text(b) if b.text == "REVEAL"), "the mask stays editable text");
}
