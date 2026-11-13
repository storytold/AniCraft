use super::*;
use crate::Session;
use anicraft_doc::sample;
use serde_json::json;
use std::path::Path;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anicraft-anim-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What the CPU renderer draws for movie frames `from..=to`, as straight RGBA8.
fn renders(doc: &anicraft_doc::Document, from: u32, to: u32, scale: f32, background: bool) -> Vec<Vec<u8>> {
    let mut view = View::stage(doc, scale);
    if !background {
        view.background = None;
    }
    (from..=to).map(|f| render(&anicraft_doc::evaluate(doc, f), &view).unwrap().0.to_rgba8()).collect()
}

/// An APNG decoded as a player shows it (the APNG specification's compositing): each frame's
/// canvas, its delay (num, den), its region and blend op, and the play count.
struct ApngFrame {
    canvas: Vec<u8>,
    delay: (u16, u16),
    rect: Rect,
    over: bool,
}

struct Apng {
    frames: Vec<ApngFrame>,
    plays: u32,
    size: (u32, u32),
}

fn decode_apng(bytes: &[u8]) -> Apng {
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = dec.read_info().unwrap();
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    let actl = info.animation_control.expect("an acTL chunk");
    assert_eq!((info.color_type, info.bit_depth), (png::ColorType::Rgba, png::BitDepth::Eight));
    let mut canvas = vec![0u8; (w * h * 4) as usize];
    let mut frames = Vec::new();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    for _ in 0..actl.num_frames {
        let out = reader.next_frame(&mut buf).unwrap();
        let fc = *reader.info().frame_control().expect("an fcTL per frame");
        assert_eq!(fc.dispose_op, png::DisposeOp::None);
        let over = fc.blend_op == png::BlendOp::Over;
        let r = Rect { x: fc.x_offset, y: fc.y_offset, w: fc.width, h: fc.height };
        assert_eq!((out.width, out.height), (r.w, r.h));
        assert!(r.x + r.w <= w && r.y + r.h <= h);
        for y in 0..r.h {
            for x in 0..r.w {
                let s = ((y * r.w + x) * 4) as usize;
                let d = (((r.y + y) * w + r.x + x) * 4) as usize;
                let src = &buf[s..s + 4];
                if over {
                    // Exact for the pixels the exporter blends: transparent or opaque.
                    assert!(src[3] == 0 || src[3] == 255, "an over-blended pixel is neither transparent nor opaque");
                    if src[3] == 255 {
                        canvas[d..d + 4].copy_from_slice(src);
                    }
                } else {
                    canvas[d..d + 4].copy_from_slice(src);
                }
            }
        }
        frames.push(ApngFrame { canvas: canvas.clone(), delay: (fc.delay_num, fc.delay_den), rect: r, over });
    }
    Apng { frames, plays: actl.num_plays, size: (w, h) }
}

/// The frames an APNG shows, one per movie frame at `ratio` fps (each delay is a whole number
/// of movie frames, exactly).
fn apng_movie(a: &Apng, (p, q): (u64, u64)) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for f in &a.frames {
        let (num, den) = (u64::from(f.delay.0), u64::from(f.delay.1));
        // num/den s = k·q/p s
        assert_eq!((num * p) % (den * q), 0, "a delay of {num}/{den} s is not a whole number of frames at {p}/{q} fps");
        for _ in 0..num * p / (den * q) {
            out.push(f.canvas.clone());
        }
    }
    out
}

/// A WebP decoded as a player shows it: each frame's canvas and duration.
/// An animated WebP's frames (canvas, milliseconds), loop count, size and whether it has alpha.
struct Webp {
    frames: Vec<(Vec<u8>, u32)>,
    looping: image_webp::LoopCount,
    size: (u32, u32),
    alpha: bool,
}

fn decode_webp(bytes: &[u8]) -> Webp {
    let mut dec = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).unwrap();
    assert!(dec.is_animated());
    let (w, h) = dec.dimensions();
    let alpha = dec.has_alpha();
    let mut buf = vec![0u8; dec.output_buffer_size().unwrap()];
    let mut frames = Vec::new();
    for _ in 0..dec.num_frames() {
        let ms = dec.read_frame(&mut buf).unwrap();
        let rgba = if alpha { buf.clone() } else { buf.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect() };
        frames.push((rgba, ms));
    }
    Webp { frames, looping: dec.loop_count(), size: (w, h), alpha }
}

/// The frames a WebP shows, one per movie frame: the frame showing when each starts.
fn webp_movie(frames: &[(Vec<u8>, u32)], n: u64, ratio: (u64, u64)) -> Vec<Vec<u8>> {
    let mut starts = Vec::new();
    let mut t = 0u64;
    for (_, ms) in frames {
        starts.push(t);
        t += u64::from(*ms);
    }
    assert_eq!(t, start_ms(n, ratio), "the running time is exact to the millisecond");
    (0..n).map(|f| {
        let at = start_ms(f, ratio);
        let i = starts.iter().rposition(|s| *s <= at).unwrap();
        frames[i].0.clone()
    })
    .collect()
}

fn same(got: &[Vec<u8>], want: &[Vec<u8>], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: frame count");
    for (k, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(g == w, "{what}: frame {} differs from the CPU renderer", k + 1);
    }
}

/// A ball that holds still for 10 frames, jumps, and holds 6 more: 16 frames, 2 pictures.
fn holds() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.ellipse", json!({"x": 40, "y": 40, "w": 60, "h": 60}));
    run(&mut s, "timeline.goto", json!({"frame": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.translate", json!({"dx": 200, "dy": 30}));
    run(&mut s, "timeline.goto", json!({"frame": 15}));
    run(&mut s, "timeline.insert_frame", json!({}));
    assert_eq!(s.frame_count(), 16);
    s
}

#[test]
fn frame_rates_are_exact_fractions() {
    assert_eq!(fps_ratio(24.0), (24, 1));
    assert_eq!(fps_ratio(29.97), (2997, 100));
    assert_eq!(fps_ratio(23.976), (2997, 125));
    assert_eq!(fps_ratio(12.5), (25, 2));
    assert_eq!(fps_ratio(f32::NAN), (24, 1));
    assert_eq!(fps_ratio(-3.0), (24, 1));
    // Millisecond starts round from the exact time, so the error never adds up.
    assert_eq!(start_ms(24, (24, 1)), 1000);
    assert_eq!(start_ms(1, (30, 1)), 33);
    assert_eq!(start_ms(2, (30, 1)), 67);
    assert_eq!(start_ms(2997, (2997, 100)), 100_000);
    assert_eq!(webp_durations(0, 3, (30, 1)), vec![100]);
    let total: u32 = (0..30).flat_map(|i| webp_durations(i, 1, (30, 1))).sum();
    assert_eq!(total, 1000);
    // APNG delays are fractions of the frame rate, reduced.
    assert_eq!(apng_delays(0, 1, (24, 1)), vec![(1, 24)]);
    assert_eq!(apng_delays(0, 6, (24, 1)), vec![(1, 4)]);
    assert_eq!(apng_delays(5, 2, (2997, 100)), vec![(200, 2997)]);
    // Longer than one fcTL holds: more pieces, adding up exactly.
    let long = apng_delays(0, 200_000, (2997, 100));
    assert!(long.len() > 1);
    let frames: u64 = long.iter().map(|&(n, d)| u64::from(n) * 2997 / (u64::from(d) * 100)).sum();
    assert_eq!(frames, 200_000);
    // A rate with a huge denominator falls back to milliseconds, still adding up.
    let odd = (100_003, 1000);
    let ms: u64 = apng_delays(0, 1000, odd).iter().map(|&(n, d)| {
        assert_eq!(d, 1000);
        u64::from(n)
    }).sum();
    assert_eq!(ms, start_ms(1000, odd));
    let hours = webp_durations(0, 100_000, (1, 1));
    assert_eq!(hours.iter().map(|&m| u64::from(m)).sum::<u64>(), 100_000_000);
    assert!(hours.iter().all(|&m| u64::from(m) <= MAX_WEBP_MS) && hours.len() > 1);
}

#[test]
fn the_changed_rectangle_is_tight() {
    let (w, h) = (8u32, 6u32);
    let a = vec![0u8; (w * h * 4) as usize];
    assert_eq!(changed(&a, &a, w, h), None);
    let mut b = a.clone();
    b[((2 * w + 3) * 4 + 1) as usize] = 9; // (3, 2)
    b[((4 * w + 6) * 4 + 3) as usize] = 9; // (6, 4)
    assert_eq!(changed(&a, &b, w, h), Some(Rect { x: 3, y: 2, w: 4, h: 3 }));
    // WebP regions start on even pixels; APNG's stay tight.
    let (r, over, px) = region(Kind::Webp, Some(&a), &b, w, h);
    assert_eq!((r, over, px.len()), (Rect { x: 2, y: 2, w: 5, h: 3 }, false, 5 * 3 * 4));
    let (r, over, _) = region(Kind::Apng, Some(&a), &b, w, h);
    assert_eq!((r, over), (Rect { x: 3, y: 2, w: 4, h: 3 }, false), "semi-transparent changes replace");
    let mut c = a.clone();
    c[((2 * w + 3) * 4) as usize..((2 * w + 3) * 4 + 4) as usize].copy_from_slice(&[1, 2, 3, 255]);
    let (r, over, px) = region(Kind::Apng, Some(&a), &c, w, h);
    assert_eq!((r, over, px), (Rect { x: 3, y: 2, w: 1, h: 1 }, true, vec![1, 2, 3, 255]), "opaque changes blend over");
}

#[test]
fn apng_frames_match_the_renderer_exactly() {
    let dir = tmp("apng");
    let mut s = Session::new(sample::demo());
    for background in [false, true] {
        let path = dir.join(format!("walk-{background}.png"));
        let v = run(&mut s, "file.export_apng", json!({"path": path.display().to_string(), "scale": 0.25, "to": 23, "background": background}));
        assert_eq!(v["frames"], 24);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(v["bytes"], bytes.len());
        let a = decode_apng(&bytes);
        assert_eq!(a.plays, 0, "loops forever by default");
        let want = renders(&s.doc, 0, 23, 0.25, background);
        assert_eq!(a.size, (320, 180));
        same(&apng_movie(&a, fps_ratio(s.doc.stage.fps)), &want, "APNG");
        // Frames after the first cover only what changed.
        assert!(a.frames.iter().skip(1).any(|f| f.rect.w * f.rect.h < 320 * 180), "dirty rectangles");
        if background {
            assert!(a.frames.iter().skip(1).all(|f| f.over), "opaque frames blend over");
        }
        // The default image is the first frame, and a plain PNG reader sees it.
        let first = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().unwrap().info().frame_control.unwrap();
        assert_eq!((first.width, first.height, first.x_offset, first.y_offset), (320, 180, 0, 0));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn webp_frames_match_the_renderer_exactly() {
    let dir = tmp("webp");
    let mut s = Session::new(sample::demo());
    for background in [false, true] {
        let path = dir.join(format!("walk-{background}.webp"));
        let v = run(&mut s, "file.export_webp", json!({"path": path.display().to_string(), "scale": 0.25, "from": 4, "to": 27, "background": background}));
        assert_eq!(v["frames"], 24);
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8, bytes.len());
        let Webp { frames, looping, size, alpha } = decode_webp(&bytes);
        assert_eq!((size, alpha), ((320, 180), !background));
        assert_eq!(looping, image_webp::LoopCount::Forever);
        let want = renders(&s.doc, 4, 27, 0.25, background);
        same(&webp_movie(&frames, 24, fps_ratio(s.doc.stage.fps)), &want, "WebP");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn identical_frames_merge_into_longer_delays() {
    let dir = tmp("holds");
    let mut s = holds();
    let ratio = fps_ratio(s.doc.stage.fps);
    let want = renders(&s.doc, 0, 15, 1.0, false);
    let png = dir.join("holds.png");
    let v = run(&mut s, "file.export_apng", json!({"path": png.display().to_string(), "loop": 3}));
    assert_eq!((v["frames"].as_u64(), v["stored_frames"].as_u64(), v["loop"].as_u64()), (Some(16), Some(2), Some(3)));
    let a = decode_apng(&std::fs::read(&png).unwrap());
    assert_eq!(a.frames.len(), 2);
    assert_eq!(a.plays, 3);
    let fps = s.doc.stage.fps;
    assert_eq!(a.frames[0].delay, apng_delays(0, 10, ratio)[0], "10 frames at {fps} fps");
    same(&apng_movie(&a, ratio), &want, "held APNG");
    let webp = dir.join("holds.webp");
    let v = run(&mut s, "file.export_webp", json!({"path": webp.display().to_string(), "loop": false}));
    assert_eq!(v["stored_frames"], 2);
    let Webp { frames, looping, .. } = decode_webp(&std::fs::read(&webp).unwrap());
    assert_eq!(frames.len(), 2);
    assert_eq!(looping, image_webp::LoopCount::Times(std::num::NonZeroU16::new(1).unwrap()));
    same(&webp_movie(&frames, 16, ratio), &want, "held WebP");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_params_are_refused_and_overwrites_confirm() {
    let dir = tmp("params");
    let mut s = holds();
    let at = |n: &str| dir.join(n).display().to_string();
    for id in ["file.export_apng", "file.export_webp"] {
        for bad in [json!({"loop": -1}), json!({"loop": 70_000}), json!({"loop": "yes"}), json!({"scale": 0}), json!({"from": 9, "to": 2}), json!({"background": 3})] {
            let mut p = bad.clone();
            p["path"] = json!(at("x.out"));
            assert!(s.execute(id, &p).is_err(), "{id} {bad}");
        }
        assert!(s.execute(id, &json!({"path": at("big.out"), "scale": 8})).is_err(), "{id}: too large");
        assert!(!Path::new(&at("x.out")).exists() && !Path::new(&at("big.out")).exists());
        let path = at(&format!("once-{id}.out"));
        run(&mut s, id, json!({"path": path, "to": 1}));
        assert!(s.execute(id, &json!({"path": path})).is_err(), "{id}: replacing a file asks first");
        run(&mut s, id, json!({"path": path, "confirm": true}));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_cancelled_publish_stops_the_export() {
    let mut s = Session::new(sample::demo());
    let job = std::sync::Arc::new(crate::publish::Control::default());
    job.cancel();
    s.job = Some(job);
    let dir = tmp("cancel");
    let path = dir.join("walk.webp");
    let e = s.execute("file.export_webp", &json!({"path": path.display().to_string(), "scale": 0.1})).unwrap_err();
    assert!(matches!(e, EngineError::Cancelled), "{e}");
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "a measurement: cargo test --release -p anicraft-engine anim::tests::sizes -- --ignored --nocapture"]
fn sizes() {
    // docs/performance.md › Animated exports: the sample, every frame, at full size and half.
    let dir = tmp("sizes");
    for scale in [1.0, 0.5] {
        let mut s = Session::new(sample::demo());
        let mut row = format!("scale {scale}:");
        for (id, ext, extra) in [
            ("file.export_gif", "gif", json!({})),
            ("file.export_apng", "png", json!({"background": true})),
            ("file.export_apng", "png", json!({})),
            ("file.export_webp", "webp", json!({"background": true})),
            ("file.export_webp", "webp", json!({})),
        ] {
            let mut p = extra.clone();
            p["path"] = json!(dir.join(format!("walk-{scale}-{}.{ext}", extra.as_object().map_or(0, |o| o.len()))).display().to_string());
            p["scale"] = json!(scale);
            p["confirm"] = json!(true);
            let t = std::time::Instant::now();
            let v = run(&mut s, id, p);
            row.push_str(&format!(" | {id} {extra}: {} KB, {} stored, {} ms", v["bytes"].as_u64().unwrap() / 1024, v["stored_frames"], t.elapsed().as_millis()));
        }
        println!("{row}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
