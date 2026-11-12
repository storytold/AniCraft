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
    // Per-character x positions split "Three" where they place characters (ADR 0070); a text
    // path whose path is missing is drawn on a line.
    let words: Vec<&str> = t.iter().map(|(_, b)| b.text.as_str()).collect();
    assert_eq!(words, ["One", "Two", "T", "h", "ree", "Four"]);
    for (k, x) in [(3, 10.0), (4, 20.0)] {
        let r = ink(&t[k].0, &t[k].1);
        assert!(r.min.x >= x - 0.5 && r.min.x < x + 4.0, "{}: {r:?}", t[k].1.text);
    }
    assert_eq!(report.missing_fonts.iter().map(String::as_str).collect::<Vec<_>>(), ["Zz Missing Face"]);
    assert!(fonts.families().contains(report.fallback_font.as_ref().unwrap()));
    assert!(report.summary().contains("Fonts not on this computer, drawn in"), "{}", report.summary());
    assert_eq!(report.skipped.get("strokes on text (left out)"), Some(&3), "once per block");
    for k in ["underlined or struck-through text (lines left out)", "text on a path whose path is missing (drawn on a line)", "outlined text without a fill (left out)"] {
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

#[test]
fn text_paths_become_text_on_a_path() {
    let Some(fonts) = fonts() else { return };
    let (t, report) = texts(
        r##"<svg width="400" height="300" font-family="sans-serif" font-size="20">
          <defs><path id="line" d="M 50 200 L 350 200"/><circle id="ring" cx="200" cy="100" r="60" transform="translate(0 10)"/></defs>
          <text><textPath href="#line">Start here</textPath></text>
          <text text-anchor="middle"><textPath xlink:href="#line" startOffset="50%">Middle</textPath></text>
          <text><textPath href="#line" startOffset="20" side="right">Under</textPath></text>
          <text><textPath href="#ring">Around the ring</textPath></text>
        </svg>"##,
        fonts,
    );
    assert!(report.skipped.is_empty(), "{report:?}");
    let words: Vec<&str> = t.iter().map(|(_, b)| b.text.as_str()).collect();
    assert_eq!(words, ["Start here", "Middle", "Under", "Around the ring"]);
    for (_, b) in &t {
        assert!(b.on_path.is_some(), "{}", b.text);
    }
    // On the line, standing on it, from its start.
    let r = ink(&t[0].0, &t[0].1);
    assert!((r.min.x - 50.0).abs() < 4.0 && r.max.y <= 206.0 && r.max.y > 195.0, "{r:?}");
    // startOffset 50% with text-anchor middle: centred on x 200.
    assert_eq!(t[1].1.align, TextAlign::Center);
    assert!((ink(&t[1].0, &t[1].1).center().x - 200.0).abs() < 3.0);
    assert!((t[1].1.on_path.as_ref().unwrap().offset - 150.0).abs() < 0.01);
    // side="right": walked backwards from the end, under the line.
    let u = ink(&t[2].0, &t[2].1);
    let on = t[2].1.on_path.as_ref().unwrap();
    assert!(on.flip && u.min.y > 195.0 && u.max.x <= 331.0, "{u:?}");
    // The ring keeps its own transform: glyphs sit round (200, 110), radius 60.
    let ring = ink(&t[3].0, &t[3].1);
    assert!(ring.min.x > 200.0 - 90.0 && ring.max.x < 200.0 + 90.0 && ring.min.y > 110.0 - 90.0 && ring.max.y < 110.0 + 90.0, "{ring:?}");
    let path = t[3].1.on_path.as_ref().unwrap().path.transformed(&t[3].0.transform).bounds();
    assert!((path.center().y - 110.0).abs() < 0.5 && (path.width() - 120.0).abs() < 0.5, "{path:?}");
}

#[test]
fn per_character_shifts_and_rotations_place_their_characters() {
    let Some(fonts) = fonts() else { return };
    let (t, report) = texts(
        r##"<svg width="400" height="300" font-family="sans-serif" font-size="20">
          <text x="10" y="50" dx="0 0 30">abcd</text>
          <text x="10" y="100" dy="0 10 10">xyz</text>
          <text x="10" y="150" rotate="0 90">ok</text>
          <text x="10 40" y="200 220">AB</text>
        </svg>"##,
        fonts,
    );
    assert!(report.skipped.is_empty(), "{report:?}");
    let words: Vec<&str> = t.iter().map(|(_, b)| b.text.as_str()).collect();
    assert_eq!(words, ["ab", "cd", "x", "y", "z", "o", "k", "A", "B"]);
    // dx 30 on "c": it starts 30 px after "ab" ends.
    let (ab, cd) = (ink(&t[0].0, &t[0].1), ink(&t[1].0, &t[1].1));
    let gap = cd.min.x - (t[0].0.transform.translation().x + t[0].1.box_width);
    assert!((gap - 30.0).abs() < 3.0, "{ab:?} {cd:?} gap {gap}");
    assert!((ab.max.y - cd.max.y).abs() < 2.0, "same baseline");
    // dy moves each later character down, cumulatively.
    let (x, y, z) = (ink(&t[2].0, &t[2].1), ink(&t[3].0, &t[3].1), ink(&t[4].0, &t[4].1));
    assert!((y.min.y - x.min.y - 10.0).abs() < 2.0 && (z.min.y - x.min.y - 20.0).abs() < 2.0, "{x:?} {y:?} {z:?}");
    assert!(y.min.x > x.max.x - 1.0, "continues after the x");
    // rotate 90 on "k": turned a quarter about its baseline origin (taller than wide, below).
    let k = ink(&t[6].0, &t[6].1);
    assert!(k.min.y > 140.0 && t[6].0.transform.m[1].abs() > 0.99, "{k:?} {:?}", t[6].0.transform);
    // Absolute x and y lists place each character.
    let b = ink(&t[8].0, &t[8].1);
    assert!(b.min.x >= 40.0 && b.min.x < 44.0 && (b.max.y - 220.0).abs() < 1.5, "{b:?}");
}
