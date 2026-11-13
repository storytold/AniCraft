//! Text import tests on self-authored XFL snippets (ADR 0014).

use crate::import::{Report, XflError, import_with, import_with_fonts};
use crate::text::GUTTER;
use anicraft_doc::{Document, Element, ElementKind, LineType, Paint, Rgba, TextAlign, TextBlock, TextKind, evaluate};
use anicraft_text::Fonts;
use std::sync::Arc;

/// The computer's fonts and an installed family, or `None` (and the test passes) on a computer
/// without fonts.
fn fonts() -> Option<(&'static Fonts, String)> {
    let f = Fonts::get();
    let family = f.resolve("")?;
    Some((f, family))
}

/// A document with these elements on its one frame.
fn doc_with(elements: &str) -> String {
    format!(r#"<DOMDocument width="550" height="400"><timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="Text"><frames><DOMFrame index="0"><elements>{elements}</elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"#)
}

fn import_doc(xml: &str, fonts: &Fonts) -> (Document, Report) {
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok((p == "DOMDocument.xml").then(|| xml.as_bytes().to_vec())) };
    import_with_fonts(&read, fonts).unwrap()
}

fn texts(doc: &Document) -> Vec<(Arc<Element>, Arc<TextBlock>)> {
    doc.timeline(0).layers[0].keyframes[0].elements.iter().filter_map(|e| if let ElementKind::Text(t) = &e.kind { Some((e.clone(), t.clone())) } else { None }).collect()
}

fn run(chars: &str, attrs: &str) -> String {
    format!("<DOMTextRun><characters>{chars}</characters><textAttrs><DOMTextAttrs {attrs}/></textAttrs></DOMTextRun>")
}

#[test]
fn static_text_keeps_its_style_matrix_and_lines() {
    let Some((fonts, family)) = fonts() else { return };
    let field = format!(
        r##"<DOMStaticText autoExpand="true" width="200" height="60"><matrix><Matrix a="2" d="2" tx="100" ty="50"/></matrix><textRuns>{}</textRuns></DOMStaticText>"##,
        run("Hello&#xD;World", &format!(r##"face="{family}" size="24" fillColor="#FF0000" alignment="center" letterSpacing="1.5" lineSpacing="4""##))
    );
    let (doc, report) = import_doc(&doc_with(&field), fonts);
    let t = texts(&doc);
    assert_eq!(t.len(), 1);
    let (el, b) = &t[0];
    assert_eq!(b.text, "Hello\nWorld", "Animate's carriage returns are line breaks");
    assert_eq!((b.font.as_str(), b.size, b.color, b.align), (family.as_str(), 24.0, Rgba::hex(0xff0000), TextAlign::Center));
    assert_eq!((b.letter_spacing, b.line_spacing, b.width, b.bold, b.italic), (1.5, 4.0, None, false, false));
    // The matrix places the field; its text sits inside the gutter.
    assert_eq!(el.transform.m, [2.0, 0.0, 0.0, 2.0, 100.0 + 2.0 * GUTTER, 50.0 + 2.0 * GUTTER]);
    // Two lines, laid out by the text engine and stored as outlines.
    let one = fonts.layout("Hello", &anicraft_text::Style { family: family.clone(), size: 24.0, line_spacing: 4.0, ..Default::default() }).unwrap();
    assert!(b.box_height > one.height + one.line_height - 0.5, "{} vs {}", b.box_height, one.height);
    assert!(!b.outline.cmds.is_empty());
    assert!(report.skipped.is_empty() && report.missing_fonts.is_empty(), "{report:?}");
    assert_eq!(report.summary(), "Imported everything in the file.");
    // It renders, red, where the matrix puts it.
    let list = evaluate(&doc, 0);
    let item = list.items.iter().find(|i| i.fill == Some(Paint::Solid(Rgba::hex(0xff0000)))).expect("the text renders");
    assert!(item.transform.translation().x > 100.0);
}

#[test]
fn fixed_width_fields_wrap_and_auto_size_ones_do_not() {
    let Some((fonts, family)) = fonts() else { return };
    let words = "one two three four five six seven eight nine ten";
    let field = |extra: &str| format!(r#"<DOMStaticText {extra} width="90"><textRuns>{}</textRuns></DOMStaticText>"#, run(words, &format!(r#"face="{family}" size="14""#)));
    let (doc, _) = import_doc(&doc_with(&(field("") + &field(r#"autoExpand="true""#))), fonts);
    let t = texts(&doc);
    assert_eq!(t[0].1.width, Some(90.0), "fixed width");
    assert!(t[0].1.box_height > 3.0 * 14.0, "wrapped onto several lines: {}", t[0].1.box_height);
    assert!(t[0].1.outline.bounds().max.x <= 92.0);
    assert_eq!(t[1].1.width, None, "auto-size");
    assert!(t[1].1.box_width > 200.0 && t[1].1.box_height < 2.0 * 14.0 * 1.5, "one line: {} × {}", t[1].1.box_width, t[1].1.box_height);
}

#[test]
fn runs_in_different_styles_become_blocks_stacked_line_by_line() {
    let Some((fonts, family)) = fonts() else { return };
    let title = format!(r##"face="{family}" size="30" bold="true" fillColor="#112233""##);
    let body = format!(r##"face="{family}" size="12" fillColor="#445566""##);
    // A title line, two body lines (the second continues in a separate run of the same style),
    // then a line that mixes both styles: most of its characters are body text.
    let runs = [run("Title&#xD;", &title), run("line one&#xD;line ", &body), run("two&#xD;", &body), run("Big", &title), run(" and mostly small", &body), run("&#xD;&#xD;", &body)].concat();
    let field = format!(r#"<DOMStaticText autoExpand="true"><matrix><Matrix tx="10" ty="20"/></matrix><textRuns>{runs}</textRuns></DOMStaticText>"#);
    let (doc, report) = import_doc(&doc_with(&field), fonts);
    let t = texts(&doc);
    let blocks: Vec<(&str, f32, bool)> = t.iter().map(|(_, b)| (b.text.as_str(), b.size, b.bold)).collect();
    assert_eq!(blocks, [("Title", 30.0, true), ("line one\nline two\nBig and mostly small", 12.0, false)], "trailing empty lines are dropped");
    assert_eq!(t[1].1.color, Rgba::hex(0x445566));
    // The body starts where the title's next line would.
    let title_layout = fonts.layout("Title", &anicraft_text::Style { family: family.clone(), size: 30.0, bold: true, ..Default::default() }).unwrap();
    let (y0, y1) = (t[0].0.transform.translation().y, t[1].0.transform.translation().y);
    assert!((y0 - (20.0 + GUTTER)).abs() < 1e-4);
    assert!((y1 - y0 - title_layout.line_height).abs() < 1e-3, "{y0} → {y1}, line height {}", title_layout.line_height);
    assert_eq!(t[0].0.transform.translation().x, t[1].0.transform.translation().x, "left-aligned blocks share the left edge");
    assert_eq!(report.skipped.get("lines with mixed text styles (drawn in their main style)"), Some(&1));
}

#[test]
fn centred_auto_size_blocks_centre_on_each_other() {
    let Some((fonts, family)) = fonts() else { return };
    let a = format!(r#"face="{family}" size="40" alignment="center""#);
    let b = format!(r#"face="{family}" size="12" alignment="center""#);
    let field = format!(r#"<DOMStaticText autoExpand="true"><textRuns>{}{}</textRuns></DOMStaticText>"#, run("WIDE HEADLINE&#xD;", &a), run("small", &b));
    let (doc, _) = import_doc(&doc_with(&field), fonts);
    let t = texts(&doc);
    let centre = |i: usize| t[i].0.transform.translation().x + t[i].1.box_width * 0.5;
    assert!((centre(0) - centre(1)).abs() < 0.01, "{} vs {}", centre(0), centre(1));
}

#[test]
fn missing_fonts_fall_back_and_are_reported() {
    let Some((fonts, family)) = fonts() else { return };
    let field = format!(
        r#"<DOMStaticText><textRuns>{}</textRuns></DOMStaticText><DOMStaticText><textRuns>{}{}</textRuns></DOMStaticText>"#,
        run("Hi", r#"face="NoSuchFace-BoldItalic" size="20""#),
        run("again", r#"face="NoSuchFace-BoldItalic" size="20""#),
        run("&#xD;mono", r#"face="ZzMissingMonoPSMT" size="20""#)
    );
    let (doc, report) = import_doc(&doc_with(&field), fonts);
    let t = texts(&doc);
    assert_eq!(t.len(), 3);
    assert_eq!((t[0].1.bold, t[0].1.italic), (true, true), "the face name says bold italic");
    assert!(fonts.families().contains(&t[0].1.font), "laid out in an installed family: {}", t[0].1.font);
    assert_eq!(report.missing_fonts.iter().map(String::as_str).collect::<Vec<_>>(), ["No Such Face", "Zz Missing Mono"]);
    assert_eq!(report.fallback_font.as_deref(), Some(family.as_str()));
    assert_eq!(report.summary(), format!("Imported. Fonts not on this computer, drawn in {family}: No Such Face, Zz Missing Mono."));
}

#[test]
fn dynamic_and_input_text_become_fields_with_their_names() {
    let Some((fonts, _)) = fonts() else { return };
    let field = format!(
        r#"<DOMDynamicText name="score" width="100" height="22" border="true" isSelectable="false"><matrix><Matrix tx="10" ty="20"/></matrix><textRuns>{}</textRuns></DOMDynamicText><DOMInputText name="answer" lineType="password" maxCharacters="8" variableName="_root.pw" width="120" height="20"><textRuns>{}</textRuns></DOMInputText><DOMInputText name="notes" lineType="multiline" width="150" height="60"/><DOMStaticText orientation="vertical left to right"><textRuns>{}</textRuns></DOMStaticText>"#,
        run("0", r#"size="16""#),
        run("type here", r#"size="16" alignment="justify""#),
        run("down", r#"size="16" indent="5" characterPosition="superscript""#)
    );
    let (doc, report) = import_doc(&doc_with(&field), fonts);
    let t = texts(&doc);
    assert_eq!(t.iter().map(|(e, _)| e.name.as_str()).collect::<Vec<_>>(), ["score", "answer", "notes", ""], "instance names are kept");
    let kinds: Vec<TextKind> = t.iter().map(|(_, b)| b.kind()).collect();
    assert_eq!(kinds, [TextKind::Dynamic, TextKind::Input, TextKind::Input, TextKind::Static]);
    // The box is the field's; the text sits in the gutter, the element at the field's matrix.
    let (score, sb) = &t[0];
    assert_eq!((sb.box_width, sb.box_height), (100.0, 22.0));
    assert_eq!(score.transform.translation(), anicraft_geom::Vec2::new(10.0, 20.0));
    assert!(sb.outline.bounds().min.x >= GUTTER - 0.5, "{:?}", sb.outline.bounds());
    let f = sb.field.as_ref().unwrap();
    assert!(f.border && f.background && !f.selectable);
    let f = t[1].1.field.as_ref().unwrap();
    assert_eq!((f.lines, f.max_chars, f.variable.as_str()), (LineType::Password, 8, "_root.pw"));
    assert_eq!(t[1].1.text, "type here", "the value is kept; it is drawn as stars");
    let n = &t[2].1;
    assert_eq!((n.text.as_str(), n.field.as_ref().map(|f| f.lines), n.box_height), ("", Some(LineType::Multiline), 60.0), "an empty input field is kept");
    assert_eq!(report.runs.get("dynamic text fields"), Some(&1));
    assert_eq!(report.runs.get("input text fields"), Some(&2));
    assert_eq!(report.kept.get("text field variables (ActionScript 1.0 and 2.0)"), Some(&1));
    for (what, n) in [("justified text (drawn left-aligned)", 1), ("vertical text (drawn horizontal)", 1), ("first-line indents", 1), ("superscript and subscript (drawn on the line)", 1)] {
        assert_eq!(report.skipped.get(what), Some(&n), "{what}: {report:?}");
    }
    assert!(report.missing_fonts.is_empty(), "no face asks for the default family");
    // Every non-interactive render draws the authored text in its box, with its border.
    let dl = evaluate(&doc, 0);
    assert!(dl.items.iter().any(|i| i.stroke.is_some()), "the border");
}

#[test]
fn without_fonts_text_is_reported_not_guessed() {
    let none = Fonts::from_faces(Vec::new());
    let field = format!(r#"<DOMStaticText><textRuns>{}</textRuns></DOMStaticText>"#, run("Hello", r#"face="ArialMT" size="20""#));
    let (doc, report) = import_doc(&doc_with(&field), &none);
    assert!(texts(&doc).is_empty());
    assert_eq!(report.skipped.get("text fields (no fonts on this computer)"), Some(&1));
    // Empty fields and fields without runs make nothing and report nothing.
    let (doc, report) = import_doc(&doc_with(r#"<DOMStaticText/><DOMStaticText><textRuns>
      <DOMTextRun><characters>   </characters></DOMTextRun></textRuns></DOMStaticText>"#), &none);
    assert!(texts(&doc).is_empty() && report.skipped.is_empty(), "{report:?}");
}

#[test]
fn hostile_text_fields_import_without_panicking() {
    let Some((fonts, _)) = fonts() else { return };
    let many: String = (0..12_000).map(|i| run("x", &format!(r#"size="{}""#, i % 7 + 1))).collect();
    let long = "y ".repeat(60_000);
    let field = format!(
        r#"<DOMStaticText width="nan" left="1e30"><textRuns>{many}</textRuns></DOMStaticText>
           <DOMStaticText width="-5"><textRuns>{}{}</textRuns></DOMStaticText>
           <DOMStaticText><textRuns><DOMTextRun><characters>z</characters><textAttrs><DOMTextAttrs size="nan" letterSpacing="1e20" lineSpacing="-1e20" fillColor="nonsense" leftMargin="-4"/></textAttrs></DOMTextRun></textRuns></DOMStaticText>"#,
        run(&long, ""),
        run(&long, "")
    );
    let start = std::time::Instant::now();
    let (doc, report) = import_doc(&doc_with(&field), fonts);
    let t = texts(&doc);
    assert!(!t.is_empty());
    assert!(t.iter().all(|(_, b)| b.text.chars().count() <= anicraft_text::MAX_CHARS && b.size.is_finite()));
    assert!(t.iter().any(|(_, b)| b.text.chars().count() > 99_000 && !b.outline.cmds.is_empty()), "the long field is laid out, cut to the limit");
    assert!(report.skipped.contains_key("text cut to 100,000 characters per field"), "{report:?}");
    assert!(start.elapsed().as_secs() < 30, "bounded work: {:?}", start.elapsed());
}

#[test]
fn import_with_uses_this_computers_fonts() {
    if fonts().is_none() {
        return;
    }
    let xml = doc_with(&format!(r#"<DOMStaticText><textRuns>{}</textRuns></DOMStaticText>"#, run("Hi", r#"size="20""#)));
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok((p == "DOMDocument.xml").then(|| xml.as_bytes().to_vec())) };
    let (doc, _) = import_with(&read).unwrap();
    assert_eq!(texts(&doc).len(), 1);
}
