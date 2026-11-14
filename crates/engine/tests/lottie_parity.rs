//! Lottie export parity (ADR 0089): the exported animation, played by a Lottie player written
//! for the test from the public format description (`lottie_player`), draws what `evaluate`
//! draws at every frame, for the sample scenes, a movie of every kind of tween and ease, and
//! the self-authored XFL corpus.
//!
//! Each frame is drawn three ways by the CPU oracle: the document itself, the eased export
//! and the frame-by-frame export of earlier builds (`LottieKeys::Frames`). Where the export
//! reports nothing lost, the eased export must match the document; where it reports a loss
//! (a symbol fading as one group is faded shape by shape, a stretched radial gradient…), it
//! must draw at least as close to the document as the frame-by-frame export does. File sizes
//! of both are printed (`--nocapture`) and the eased one must be smaller.

mod lottie_player;

use anicraft_engine::doc::{ColorEffect, CustomEase, Document, Easing, Element, ElementId, ElementKind, Keyframe, LayerId, MotionTween, Paint, Property, PropertyKey, Rgba, Shape, Stroke, Track, Tween, TweenKind, sample};
use anicraft_engine::geom::{Affine, Path, Vec2};
use anicraft_engine::render::lottie::{LottieKeys, STEPPED, to_lottie_keyed};
use anicraft_engine::render::{View, render};
use anicraft_engine::text::Fonts;
use serde_json::Value;
use std::sync::Arc;

/// A colour channel counts as different above this (out of 255): anti-aliasing noise from
/// sub-thousandth-of-a-pixel differences stays below it.
const CHANNEL: u8 = 3;

/// How a drawing differs from the document's: pixels with a channel off by more than
/// [`CHANNEL`], and the largest difference.
fn diff(a: &[u8], b: &[u8]) -> (usize, u8) {
    let mut count = 0;
    let mut worst = 0u8;
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        worst = worst.max(d);
        if d > CHANNEL {
            count += 1;
        }
    }
    (count, worst)
}

fn draw(dl: &anicraft_engine::doc::DisplayList, view: &View) -> Vec<u8> {
    render(dl, view).unwrap().0.to_rgba8()
}

/// Plays both exports of `doc` against the oracle at every frame; returns (eased, frames)
/// sizes in bytes.
fn check(name: &str, doc: &Document, scale: f32) -> (usize, usize) {
    let last = doc.frame_count().saturating_sub(1);
    let eased = to_lottie_keyed(doc, name, 0, last, true, LottieKeys::Eased);
    let frames = to_lottie_keyed(doc, name, 0, last, true, LottieKeys::Frames);
    let lossless = eased.report.keys().all(|k| k == STEPPED);
    let (ev, fv): (Value, Value) = (serde_json::from_str(&eased.json).unwrap(), serde_json::from_str(&frames.json).unwrap());
    let view = View::stage(doc, scale);
    let pixels = (view.width * view.height) as usize;
    for f in 0..=last {
        let want = draw(&anicraft_engine::doc::evaluate(doc, f), &view);
        let got = draw(&lottie_player::frame(&ev, f64::from(f)), &view);
        let (n, worst) = diff(&got, &want);
        if lossless {
            // Exact but for a few edge pixels (a thousandth of the frame).
            assert!(n * 1000 <= pixels, "{name}, frame {f}: {n} of {pixels} pixels differ (worst {worst}); report {:?}", eased.report);
        } else {
            let old = draw(&lottie_player::frame(&fv, f64::from(f)), &view);
            let (m, _) = diff(&old, &want);
            assert!(n <= m + pixels / 1000, "{name}, frame {f}: eased {n} vs frame by frame {m} pixels off; report {:?}", eased.report);
        }
    }
    println!("LOTTIE {name}: {} frames, eased {} bytes, frame by frame {} bytes ({:.0} % smaller), report {:?}", last + 1, eased.json.len(), frames.json.len(), 100.0 * (1.0 - eased.json.len() as f64 / frames.json.len() as f64), eased.report);
    assert!(eased.json.len() <= frames.json.len(), "{name}: eased {} > frame by frame {}", eased.json.len(), frames.json.len());
    (eased.json.len(), frames.json.len())
}

fn shape_el(id: u64, m: Affine, shape: Shape) -> Arc<Element> {
    Arc::new(Element { id: ElementId(id), name: String::new(), transform: m, color: ColorEffect::default(), kind: ElementKind::Shape(shape) })
}

/// A two-second movie, one layer per ease: classic tweens moving, turning, growing, skewing
/// and changing colour with every kind of ease (one cubic, several pieces, none exact), a
/// stroked shape growing evenly, a shape tween between compatible outlines, property groups
/// eased their own way, and a motion tween keyed on x, y, rotation and scale apart.
fn tweens_movie() -> Document {
    let mut doc = Document::default();
    doc.stage.width = 480;
    doc.stage.height = 360;
    let custom = Easing::Custom(Arc::new(CustomEase::from_flat(&[(0.0, 0.0), (0.2, 1.4), (0.3, 1.4), (0.4, 1.4), (0.6, 0.5), (0.8, 1.0), (1.0, 1.0)].map(|(x, y)| Vec2::new(x, y))).unwrap()));
    let eases = [Easing::Linear, Easing::Classic(-80.0), Easing::Classic(100.0), Easing::QuadInOut, Easing::BackOut, Easing::BounceOut, custom.clone(), Easing::ElasticOut, Easing::SineInOut, Easing::CubicBezier(0.2, 0.9, 0.3, 1.3)];
    let tl = doc.timeline_mut(0);
    tl.layers.clear();
    for (i, e) in eases.iter().enumerate() {
        let y = 20.0 + 30.0 * i as f32;
        let fill = Paint::Solid(Rgba::new(0.1 * i as f32, 0.5, 1.0 - 0.08 * i as f32, 1.0));
        let shape = Shape::filled(Path::rect(-10.0, -8.0, 20.0, 16.0), fill);
        let a = Affine::translate(30.0, y);
        let b = Affine::translate(400.0, y + 10.0).mul(&Affine::rotate(1.0 + i as f32 * 0.4)).mul(&Affine::scale(1.5, 0.75));
        let mut l = anicraft_engine::doc::Layer::new(LayerId(10 + i as u64), format!("ease {i}"));
        let mut k0 = Keyframe::new(0, 30);
        k0.elements = vec![shape_el(100 + i as u64, a, shape.clone())];
        let mut tw = Tween::new(e.clone(), TweenKind::Classic);
        if i == 1 {
            // Position eased one way, rotation another, scale linear.
            tw.groups = vec![(anicraft_engine::doc::EaseGroup::Rotation, Easing::BackOut), (anicraft_engine::doc::EaseGroup::Scale, Easing::Linear)];
        }
        k0.tween = Some(tw);
        let mut k1 = Keyframe::new(30, 18);
        let mut end = shape.clone();
        end.fill = Some(Paint::Solid(Rgba::new(1.0, 0.2 * (i % 5) as f32, 0.1, 0.6)));
        k1.elements = vec![shape_el(100 + i as u64, b, end)];
        l.keyframes = vec![Arc::new(k0), Arc::new(k1)];
        tl.layers.push(Arc::new(l));
    }
    // A stroked circle growing evenly (its stroke widens with it), then shrinking linearly.
    let mut ring = Shape::filled(Path::ellipse(0.0, 0.0, 12.0, 12.0), Paint::Solid(Rgba::hex(0xffcc00)));
    ring.stroke = Some(Stroke::new(Rgba::hex(0x223344), 3.0));
    let mut l = anicraft_engine::doc::Layer::new(LayerId(40), "ring");
    let mut k0 = Keyframe::new(0, 20);
    k0.elements = vec![shape_el(140, Affine::translate(240.0, 330.0), ring.clone())];
    k0.tween = Some(Tween::new(Easing::Classic(60.0), TweenKind::Classic));
    let mut k1 = Keyframe::new(20, 20);
    k1.elements = vec![shape_el(140, Affine::translate(240.0, 320.0).mul(&Affine::scale(2.0, 2.0)), ring.clone())];
    k1.tween = Some(Tween::new(Easing::Linear, TweenKind::Classic));
    let mut k2 = Keyframe::new(40, 8);
    k2.elements = vec![shape_el(140, Affine::translate(100.0, 330.0).mul(&Affine::rotate(-0.5)), ring)];
    l.keyframes = vec![Arc::new(k0), Arc::new(k1), Arc::new(k2)];
    tl.layers.push(Arc::new(l));
    // A shape tween between outlines of the same structure, eased.
    let mut l = anicraft_engine::doc::Layer::new(LayerId(41), "morph");
    let mut k0 = Keyframe::new(0, 24);
    k0.elements = vec![shape_el(141, Affine::IDENTITY, Shape::filled(Path::rect(380.0, 300.0, 40.0, 40.0), Paint::Solid(Rgba::hex(0x884488))))];
    k0.tween = Some(Tween::new(Easing::CubicOut, TweenKind::Shape));
    let mut k1 = Keyframe::new(24, 24);
    k1.elements = vec![shape_el(141, Affine::IDENTITY, Shape::filled(Path::rect(400.0, 280.0, 70.0, 20.0), Paint::Solid(Rgba::hex(0x44aa88))))];
    l.keyframes = vec![Arc::new(k0), Arc::new(k1)];
    tl.layers.push(Arc::new(l));
    // A motion tween: x, y, rotation and scale keyed at different frames with their own eases.
    let mut l = anicraft_engine::doc::Layer::new(LayerId(42), "motion");
    let mut k0 = Keyframe::new(0, 48);
    k0.elements = vec![shape_el(142, Affine::translate(60.0, 300.0), Shape::filled(Path::rect(-6.0, -6.0, 12.0, 12.0), Paint::Solid(Rgba::hex(0x00aa44))))];
    let key = |frame: u32, value: f32, ease: Easing| PropertyKey { frame, value, ease };
    let mut m = MotionTween::new(ElementId(142));
    m.tracks = vec![
        Track { property: Property::X, keys: vec![key(0, 60.0, Easing::QuadIn), key(20, 200.0, custom), key(47, 300.0, Easing::Linear)] },
        Track { property: Property::Y, keys: vec![key(0, 300.0, Easing::Linear), key(30, 250.0, Easing::BackInOut), key(47, 320.0, Easing::Linear)] },
        Track { property: Property::Rotation, keys: vec![key(0, 0.0, Easing::Classic(-40.0)), key(47, 540.0, Easing::Linear)] },
        Track { property: Property::ScaleX, keys: vec![key(10, 100.0, Easing::Linear), key(40, 250.0, Easing::Linear)] },
        Track { property: Property::ScaleY, keys: vec![key(10, 100.0, Easing::Linear), key(40, 250.0, Easing::Linear)] },
    ];
    k0.motion = Some(m);
    l.keyframes = vec![Arc::new(k0)];
    tl.layers.push(Arc::new(l));
    doc
}

#[test]
fn the_sample_scenes_play_as_evaluated() {
    for (name, doc) in [("demo", sample::demo()), ("interactive", sample::interactive())] {
        let scale = 320.0 / doc.stage.width as f32;
        check(name, &doc, scale.min(1.0));
    }
}

#[test]
fn every_tween_and_ease_plays_as_evaluated_with_few_keys() {
    let doc = tweens_movie();
    let (eased, frames) = check("tweens", &doc, 0.5);
    // A layer per ease: the exact ones are a few keys each.
    let e = to_lottie_keyed(&doc, "tweens", 0, doc.frame_count() - 1, false, LottieKeys::Eased);
    let v: Value = serde_json::from_str(&e.json).unwrap();
    let tr = |name: &str| -> Value {
        let l = v["layers"].as_array().unwrap().iter().find(|l| l["nm"] == name).unwrap_or_else(|| panic!("{name}"));
        l["shapes"][0]["it"].as_array().unwrap().last().unwrap().clone()
    };
    let count = |p: &Value| if p["a"] == 1 { p["k"].as_array().unwrap().len() } else { 1 };
    for (name, keys) in [("ease 0", 2), ("ease 1", 2), ("ease 2", 2), ("ease 3", 3), ("ease 4", 2), ("ease 5", 8), ("ease 6", 3), ("ease 9", 2)] {
        assert_eq!(count(&tr(name)["p"]), keys, "{name}: {}", tr(name)["p"]);
        assert_eq!(count(&tr(name)["r"]), if name == "ease 1" { 2 } else { keys }, "{name} rotation");
    }
    // Elastic and Sine: a key per frame while they move, and reported.
    assert!(count(&tr("ease 7")["p"]) >= 29);
    assert!(e.report.contains_key(STEPPED), "{:?}", e.report);
    assert!(eased * 2 < frames, "eased {eased} vs frame by frame {frames}");
}

fn corpus_doc(case: &str) -> Document {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../xfl/tests/corpus").join(case);
    let read = |p: &str| -> Result<Option<Vec<u8>>, anicraft_xfl::XflError> {
        match std::fs::read(dir.join(p)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    };
    anicraft_xfl::import_with_fonts(&read, &Fonts::from_faces(Vec::new())).unwrap_or_else(|e| panic!("{case}: {e}")).0
}

#[test]
fn the_xfl_corpus_plays_as_evaluated() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../xfl/tests/corpus");
    let mut cases: Vec<String> = std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    cases.sort();
    assert!(cases.len() >= 10);
    let (mut eased, mut frames) = (0, 0);
    for case in &cases {
        let (e, f) = check(case, &corpus_doc(case), 0.5);
        eased += e;
        frames += f;
    }
    println!("LOTTIE corpus: eased {eased} bytes, frame by frame {frames} bytes");
}


#[test]
#[ignore = "timing; run with --ignored --nocapture"]
fn zz_timing() {
    // A minute of Elastic: nothing fits, every frame is tried against every ease.
    let mut long = Document::default();
    {
        use anicraft_engine::doc::{Keyframe, Tween, TweenKind};
        let sq = |x: f32| shape_el(9, Affine::translate(x, 50.0), Shape::filled(Path::rect(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba::BLACK)));
        let mut k0 = Keyframe::new(0, 1440);
        k0.elements = vec![sq(0.0)];
        k0.tween = Some(Tween::new(Easing::ElasticInOut, TweenKind::Classic));
        let mut k1 = Keyframe::new(1440, 1);
        k1.elements = vec![sq(300.0)];
        let l = Arc::make_mut(&mut long.timeline_mut(0).layers[0]);
        l.keyframes = vec![Arc::new(k0), Arc::new(k1)];
        let mut other = Tween::new(Easing::CubicBezier(0.3, 0.1, 0.2, 1.0), TweenKind::Classic);
        other.groups = vec![(anicraft_engine::doc::EaseGroup::Rotation, Easing::BounceInOut), (anicraft_engine::doc::EaseGroup::Scale, Easing::BackInOut)];
        let mut k = Keyframe::new(0, 1);
        k.tween = Some(other);
        let mut l2 = anicraft_engine::doc::Layer::new(LayerId(77), "eases");
        l2.keyframes = vec![Arc::new(k)];
        long.timeline_mut(0).layers.push(Arc::new(l2));
    }
    for (n, doc) in [("stress", sample::stress(20, 10, 240)), ("tweens", tweens_movie()), ("elastic minute", long)] {
        let last = doc.frame_count() - 1;
        for keys in [LottieKeys::Frames, LottieKeys::Eased] {
            let t = std::time::Instant::now();
            let e = to_lottie_keyed(&doc, n, 0, last, false, keys);
            println!("TIME {n} {keys:?}: {} frames, {:.1} ms, {} bytes", last + 1, t.elapsed().as_secs_f64() * 1000.0, e.json.len());
        }
    }
}
