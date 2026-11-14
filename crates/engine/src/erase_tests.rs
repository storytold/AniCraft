//! The Eraser's commands (ADR 0034).

use crate::Session;
use crate::cmds_erase::measure_els;
use anicraft_doc::{ElementKind, Paint, Rgba, Shape};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn blank(object: bool) -> Session {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": object}));
    s
}

/// The keyframe's shapes, bottom to top, with their transforms.
fn shapes(s: &Session) -> Vec<(Shape, Affine)> {
    s.visible_elements()
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => Some((sh.clone(), e.transform)),
            _ => None,
        })
        .collect()
}

fn fill_of(sh: &Shape) -> Option<Rgba> {
    match sh.fill {
        Some(Paint::Solid(c)) => Some(c),
        _ => None,
    }
}

/// The fill colour covering stage point `p` (the topmost filled shape containing it).
fn top_at(s: &Session, p: Vec2) -> Option<Rgba> {
    shapes(s).iter().rev().filter(|(sh, _)| sh.fill.is_some()).find(|(sh, m)| sh.path.contains(p, m, sh.fill_rule)).and_then(|(sh, _)| fill_of(sh))
}

/// Total raw fill area and line length on the active layer.
fn measure(s: &Session) -> (f64, f64) {
    measure_els(&s.visible_elements())
}

fn stroke(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
    (0..=30).map(|i| i as f32 / 30.0).map(|t| [x0 + (x1 - x0) * t, y0 + (y1 - y0) * t]).collect()
}

const BLUE: &str = "#0000ff";
const RED: &str = "#ff0000";
const BLACK: &str = "#000000";

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// A raw 200 × 100 blue rectangle at (100, 100), outlined in black 4 wide when `outlined`.
fn rect(outlined: bool) -> Session {
    let mut s = blank(false);
    let stroke = if outlined { json!(BLACK) } else { Value::Null };
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "fill": BLUE, "stroke": stroke, "stroke_width": 4}));
    run(&mut s, "edit.deselect", json!({}));
    s
}

#[test]
fn erasing_a_band_out_of_a_rectangle_leaves_the_analytic_area() {
    let mut s = rect(false);
    assert!(near(measure(&s).0, 20_000.0, 0.01));
    let steps = s.history_len();
    // A round tip 20 wide straight down through the middle: its ends are outside the shape.
    let r = run(&mut s, "draw.erase", json!({"points": stroke(200.0, 60.0, 200.0, 240.0), "size": 20}));
    assert_eq!(r["erased"], true);
    let (area, _) = measure(&s);
    assert!(near(area, 18_000.0, 40.0), "area {area}, want 20000 − 20 × 100");
    let blues = shapes(&s).iter().filter(|(sh, _)| fill_of(sh) == Some(Rgba::hex(0x0000ff))).count();
    assert_eq!(blues, 2, "the band cuts the rectangle in two");
    assert!(shapes(&s).iter().all(|(sh, _)| sh.merge), "what is left is raw");
    assert_eq!(top_at(&s, Vec2::new(200.0, 150.0)), None);
    assert_eq!(top_at(&s, Vec2::new(150.0, 150.0)), Some(Rgba::hex(0x0000ff)));
    // The CPU renderer agrees (the oracle): the stage shows through the band.
    let img = anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
    let bg = s.doc.stage.background;
    let (gap, kept) = (img.pixel(200, 150).unwrap(), img.pixel(150, 150).unwrap());
    assert!((gap[0] - bg.r).abs() < 0.02 && (gap[2] - bg.b).abs() < 0.02, "{gap:?}");
    assert!(kept[2] > 0.9 && kept[0] < 0.1, "{kept:?}");
    // One stroke, one undo step.
    assert_eq!(s.history_len(), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert!(near(measure(&s).0, 20_000.0, 0.01));
}

#[test]
fn erase_modes_take_what_animate_takes() {
    let band = stroke(50.0, 150.0, 350.0, 150.0);
    // Fill 200 × 100 = 20000, lines 2 × (200 + 100) = 600; the band is 20 tall across it all.
    let after = |mode: &str| {
        let mut s = rect(true);
        let r = run(&mut s, "draw.erase", json!({"points": band, "size": 20, "mode": mode}));
        assert_eq!(r["erased"], true, "{mode}");
        measure(&s)
    };
    let (area, length) = after("normal");
    assert!(near(area, 16_000.0, 40.0) && near(length, 560.0, 1.0), "normal: {area} {length}");
    let (area, length) = after("fills");
    assert!(near(area, 16_000.0, 40.0) && near(length, 600.0, 0.01), "fills keeps lines: {area} {length}");
    let (area, length) = after("lines");
    assert!(near(area, 20_000.0, 0.01) && near(length, 560.0, 1.0), "lines keeps fills: {area} {length}");
}

/// Blue at x 100..200 and red at 200..300 (raw, side by side), no outlines.
fn two_fills() -> Session {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 100, "fill": BLUE, "stroke": null}));
    run(&mut s, "draw.rect", json!({"x": 200, "y": 100, "w": 100, "h": 100, "fill": RED, "stroke": null}));
    run(&mut s, "edit.deselect", json!({}));
    s
}

fn fill_area(s: &Session, c: u32) -> f64 {
    let els: Vec<_> = s.visible_elements().into_iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if fill_of(sh) == Some(Rgba::hex(c)))).collect();
    measure_els(&els).0
}

#[test]
fn erase_selected_fills_and_erase_inside_keep_the_other_fills() {
    let band = stroke(50.0, 150.0, 350.0, 150.0);
    // Selected fills: only the selected (blue) one loses the band, and what's left stays selected.
    let mut s = two_fills();
    let r = run(&mut s, "draw.erase", json!({"points": band, "size": 20, "mode": "selected_fills"}));
    assert_eq!(r["erased"], false, "nothing selected, nothing erased");
    let blue = s.visible_elements().iter().find(|e| matches!(&e.kind, ElementKind::Shape(sh) if fill_of(sh) == Some(Rgba::hex(0x0000ff)))).map(|e| e.id.0).unwrap();
    run(&mut s, "element.select", json!({"ids": [blue]}));
    run(&mut s, "draw.erase", json!({"points": band, "size": 20, "mode": "selected_fills"}));
    assert!(near(fill_area(&s, 0x0000ff), 8_000.0, 30.0) && near(fill_area(&s, 0xff0000), 10_000.0, 0.01), "{} {}", fill_area(&s, 0x0000ff), fill_area(&s, 0xff0000));
    assert_eq!(s.selection.len(), 2, "both blue pieces stay selected");
    // Inside: only the fill the stroke starts on (a 50 × 20 band and the round start's half disc).
    let mut s = two_fills();
    run(&mut s, "draw.erase", json!({"points": stroke(150.0, 150.0, 350.0, 150.0), "size": 20, "mode": "inside"}));
    let want = 10_000.0 - 1_000.0 - std::f64::consts::PI * 50.0;
    assert!(near(fill_area(&s, 0x0000ff), want, 20.0) && near(fill_area(&s, 0xff0000), 10_000.0, 0.01), "{} {}", fill_area(&s, 0x0000ff), fill_area(&s, 0xff0000));
    // Starting off the artwork erases nothing, and leaves no undo step.
    let steps = s.history_len();
    let r = run(&mut s, "draw.erase", json!({"points": stroke(50.0, 150.0, 350.0, 150.0), "size": 20, "mode": "inside"}));
    assert_eq!(r["erased"], false);
    assert_eq!(s.history_len(), steps);
}

#[test]
fn drawing_objects_and_symbols_are_not_erased() {
    let mut s = blank(true);
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "fill": BLUE}));
    run(&mut s, "draw.ellipse", json!({"x": 100, "y": 220, "w": 200, "h": 60, "fill": RED}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Disc"}));
    let (doc, steps) = (s.doc.clone(), s.history_len());
    let r = run(&mut s, "draw.erase", json!({"points": stroke(200.0, 60.0, 200.0, 300.0), "size": 30}));
    assert_eq!(r["erased"], false);
    assert_eq!(s.doc, doc, "objects and symbols are untouched");
    assert_eq!(s.history_len(), steps, "no undo step");
    // Raw shapes beside them are erased, the object stays whole.
    run(&mut s, "draw.rect", json!({"x": 320, "y": 100, "w": 100, "h": 100, "fill": BLUE, "stroke": null, "object": false}));
    run(&mut s, "draw.erase", json!({"points": stroke(150.0, 150.0, 450.0, 150.0), "size": 20}));
    let objects = shapes(&s).into_iter().filter(|(sh, _)| !sh.merge).collect::<Vec<_>>();
    assert_eq!(objects.len(), 1, "the rectangle object is whole");
    assert!(near(measure_els(&s.visible_elements()).0, 8_000.0, 30.0), "only the raw square lost the band");
}

#[test]
fn the_faucet_deletes_a_whole_fill_or_line() {
    let mut s = rect(true);
    let lines = shapes(&s).iter().filter(|(sh, _)| sh.stroke.is_some()).count();
    let r = run(&mut s, "draw.faucet", json!({"x": 150, "y": 150}));
    assert_eq!(r["kind"], "fill");
    assert!(shapes(&s).iter().all(|(sh, _)| sh.fill.is_none()), "the fill is gone, its outline stays");
    let r = run(&mut s, "draw.faucet", json!({"x": 100, "y": 150}));
    assert_eq!(r["kind"], "line");
    assert_eq!(shapes(&s).len(), lines - 1, "one side of the outline is gone");
    assert!(s.execute("draw.faucet", &json!({"x": 600, "y": 600})).is_err(), "nothing there");
    // Drawing objects are kept.
    run(&mut s, "draw.rect", json!({"x": 400, "y": 100, "w": 50, "h": 50, "fill": RED, "object": true}));
    assert!(s.execute("draw.faucet", &json!({"x": 425, "y": 125})).is_err());
}

#[test]
fn erase_everything_asks_first_and_keeps_locked_layers() {
    let mut s = rect(true);
    run(&mut s, "layer.new", json!({"name": "Kept"}));
    run(&mut s, "draw.ellipse", json!({"x": 0, "y": 0, "w": 50, "h": 50, "fill": RED, "object": true}));
    run(&mut s, "layer.toggle_lock", json!({}));
    let c = s.confirmation("draw.erase_all", &json!({})).expect("it asks");
    assert!(!c.irreversible, "undoable");
    assert!(c.message.contains("5 elements on 1 layer"), "{}", c.message);
    let r = run(&mut s, "draw.erase_all", json!({}));
    assert_eq!(r["erased"], 5);
    let tl = s.doc.timeline(0);
    let counts: Vec<usize> = tl.layers.iter().map(|l| l.keyframe_at(0).map_or(0, |k| k.elements.len())).collect();
    assert!(counts.contains(&0) && counts.contains(&1), "{counts:?}");
    assert!(s.confirmation("draw.erase_all", &json!({})).is_none(), "nothing left to ask about");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.timeline(0).layers.iter().map(|l| l.keyframe_at(0).map_or(0, |k| k.elements.len())).sum::<usize>(), 6);
}

#[test]
fn erased_drawings_round_trip_through_the_file_format() {
    let mut s = rect(true);
    run(&mut s, "draw.erase", json!({"points": [[150, 150, 0.3], [200, 160, 1.0], [260, 140, 0.6]], "size": 24}));
    let json = crate::format::to_json(&s.doc).unwrap();
    assert_eq!(crate::format::from_json(&json).unwrap(), s.doc);
}

#[test]
fn bad_params_are_errors_and_plug_ins_may_erase() {
    let mut s = rect(true);
    for p in [
        json!({}),
        json!({"points": []}),
        json!({"points": [[0, 0]], "size": 0}),
        json!({"points": [[0, 0]], "mode": "behind"}),
        json!({"points": [[0, 0]], "shape": "star"}),
        json!({"points": [[0, 0], [1e7, 0]]}),
    ] {
        assert!(s.execute("draw.erase", &p).is_err(), "{p}");
    }
    assert!(s.execute("draw.faucet", &json!({"x": "a"})).is_err());
    for id in ["draw.erase", "draw.faucet", "draw.erase_all", "paint.pick", "paint.apply_style"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}

// ----- ADR 0093: gradients and width profiles through the merge, erasing inside symbols --------

#[test]
fn an_erased_gradient_keeps_its_gradient_in_both_pieces_where_it_was() {
    let mut s = blank(false);
    let stops = vec![anicraft_doc::GradientStop { offset: 0.0, color: Rgba::hex(0xff0000) }, anicraft_doc::GradientStop { offset: 1.0, color: Rgba::hex(0x0000ff) }];
    let paint = Paint::linear(Vec2::new(100.0, 100.0), Vec2::new(300.0, 100.0), stops);
    s.style.fill_paint = Some(paint.clone());
    s.style.lock_fill = true;
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "stroke": null}));
    run(&mut s, "edit.deselect", json!({}));
    let r = run(&mut s, "draw.erase", json!({"points": stroke(200.0, 60.0, 200.0, 240.0), "size": 20}));
    assert_eq!(r["erased"], true);
    let pieces = shapes(&s);
    assert_eq!(pieces.len(), 2, "cut in two");
    for (sh, m) in &pieces {
        let f = sh.fill.as_ref().map(|f| f.transformed(m)).expect("filled");
        assert!(crate::merge::same_paint(&f, &paint), "the gradient stays in the same place on the stage: {f:?}");
    }
    assert!(near(measure(&s).0, 18_000.0, 40.0));
    // The renderer agrees: left red, right blue, the band empty.
    let img = anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
    let (l, r) = (img.pixel(110, 150).unwrap(), img.pixel(290, 150).unwrap());
    assert!(l[0] > 0.85 && l[2] < 0.15 && r[2] > 0.85 && r[0] < 0.15, "{l:?} {r:?}");
}

/// The raw strokes on the active layer with their profiles, as (start x, end x, profile).
fn profiled(s: &Session) -> Vec<(f32, f32, Option<anicraft_doc::WidthProfile>)> {
    shapes(s)
        .into_iter()
        .filter_map(|(sh, m)| {
            let st = sh.stroke.as_ref()?;
            let ends: Vec<Vec2> = sh.path.anchors().iter().map(|p| m.apply(p.1)).collect();
            Some((ends.first()?.x, ends.last()?.x, st.profile.clone()))
        })
        .collect()
}

#[test]
fn a_variable_width_stroke_split_by_the_eraser_keeps_its_profile_per_piece() {
    let mut s = blank(false);
    s.style.set_profile(anicraft_doc::ProfilePreset::TaperBoth);
    let source = anicraft_doc::ProfilePreset::TaperBoth.profile().unwrap();
    run(&mut s, "draw.line", json!({"x0": 100, "y0": 100, "x1": 300, "y1": 100, "stroke": BLACK, "stroke_width": 12}));
    run(&mut s, "edit.deselect", json!({}));
    assert_eq!(profiled(&s).len(), 1, "a variable-width line is raw (ADR 0093)");
    let r = run(&mut s, "draw.erase", json!({"points": stroke(150.0, 60.0, 150.0, 140.0), "size": 20}));
    assert_eq!(r["erased"], true);
    let pieces = profiled(&s);
    assert_eq!(pieces.len(), 2, "{pieces:?}");
    for (x0, x1, prof) in &pieces {
        let prof = prof.as_ref().expect("each piece keeps a profile");
        // Every place on the piece has the width it had on the line.
        for k in 0..=10 {
            let u = k as f32 / 10.0;
            let x = x0 + (x1 - x0) * u;
            let want = source.at((x - 100.0) / 200.0);
            let got = prof.at(u);
            assert!((got.0 - want.0).abs() < 0.06 && (got.1 - want.1).abs() < 0.06, "at x {x}: {got:?} vs {want:?} ({x0}..{x1})");
        }
    }
    // The left piece starts at a point and ends wide; erasing nothing changes nothing.
    let doc = s.doc.clone();
    let r = run(&mut s, "draw.erase", json!({"points": stroke(150.0, 300.0, 150.0, 340.0), "size": 20}));
    assert_eq!(r["erased"], false);
    assert_eq!(s.doc, doc);
    // A line across a variable-width line cuts it too, the profile following each piece.
    let mut t = blank(false);
    t.style.set_profile(anicraft_doc::ProfilePreset::TaperBoth);
    run(&mut t, "draw.line", json!({"x0": 100, "y0": 100, "x1": 300, "y1": 100, "stroke": BLACK, "stroke_width": 12}));
    t.style.set_profile(anicraft_doc::ProfilePreset::Uniform);
    run(&mut t, "draw.line", json!({"x0": 200, "y0": 0, "x1": 200, "y1": 200, "stroke": RED, "stroke_width": 2}));
    let cut: Vec<_> = profiled(&t).into_iter().filter(|p| p.2.is_some()).collect();
    assert_eq!(cut.len(), 2, "{cut:?}");
    // Merging the pieces again (a drop) keeps them as they are.
    let before = profiled(&t);
    let ids: Vec<u64> = t.visible_elements().iter().map(|e| e.id.0).collect();
    t.select_only(ids.iter().map(|i| anicraft_doc::ElementId(*i)));
    run(&mut t, "draw.drop", json!({}));
    let after = profiled(&t);
    assert_eq!(after.len(), before.len());
    for (a, b) in after.iter().filter(|p| p.2.is_some()).zip(before.iter().filter(|p| p.2.is_some())) {
        let (pa, pb) = (a.2.as_ref().unwrap(), b.2.as_ref().unwrap());
        for k in 0..=10 {
            let u = k as f32 / 10.0;
            assert!((pa.at(u).0 - pb.at(u).0).abs() < 1e-3, "remerged profile drifted");
        }
    }
    // And the file keeps them.
    let json = crate::format::to_json(&s.doc).unwrap();
    assert_eq!(crate::format::from_json(&json).unwrap(), s.doc);
}

#[test]
fn the_eraser_works_inside_an_open_symbol_and_says_how_on_the_stage() {
    let mut s = blank(false);
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 200, "h": 100, "fill": BLUE, "stroke": null}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Card"}));
    let inst = s.selection.id_list().first().copied().expect("the instance is selected");
    run(&mut s, "edit.deselect", json!({}));
    // On the stage, the symbol is kept, and the reason says how to erase inside it.
    let (doc, steps) = (s.doc.clone(), s.history_len());
    let r = run(&mut s, "draw.erase", json!({"points": stroke(200.0, 60.0, 200.0, 240.0), "size": 20}));
    assert_eq!(r["erased"], false);
    assert_eq!(r["kept"]["symbols"], 1);
    assert!(r["reason"].as_str().unwrap().contains("double-click"), "{r}");
    assert_eq!((s.doc.clone(), s.history_len()), (doc, steps), "no change, no undo step");
    // Opened in place, its raw shapes erase, in the symbol's own space.
    run(&mut s, "symbol.edit", json!({"id": inst}));
    let to_symbol = s.editing.last().map(|l| l.transform).and_then(|m| m.inverse()).unwrap();
    let pts: Vec<[f32; 2]> = stroke(200.0, 60.0, 200.0, 240.0).into_iter().map(|[x, y]| to_symbol.apply(Vec2::new(x, y))).map(|p| [p.x, p.y]).collect();
    let r = run(&mut s, "draw.erase", json!({"points": pts, "size": 20}));
    assert_eq!(r["erased"], true, "{r}");
    run(&mut s, "symbol.exit", json!({"all": true}));
    // The instance on the stage shows the cut (the renderer, the oracle).
    let img = anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
    let bg = s.doc.stage.background;
    let (gap, kept) = (img.pixel(200, 150).unwrap(), img.pixel(150, 150).unwrap());
    assert!((gap[2] - bg.b).abs() < 0.02 && (gap[0] - bg.r).abs() < 0.02, "{gap:?}");
    assert!(kept[2] > 0.9, "{kept:?}");
    // One undo step puts the symbol back whole.
    run(&mut s, "edit.undo", json!({}));
    let img = anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
    assert!(img.pixel(200, 150).unwrap()[2] > 0.9);
}
