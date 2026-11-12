#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use super::*;
use crate::{Element, ElementId, Keyframe, Layer, LayerId, LayerKind, Scene, Symbol};
use anicraft_audio::{MixOptions, mix};
use anicraft_audio::mix::lerp;

/// A sound of `n` frames at 8 kHz whose sample `i` is `i / 1000`.
fn ramp_sound(id: u64, n: usize) -> Sound {
    let s: Vec<f32> = (0..n).map(|i| i as f32 / 1000.0).collect();
    Sound::from_data(SoundId(id), "ramp.wav", anicraft_audio::wav::encode(8000, 1, &s, anicraft_audio::wav::Bits::Float32)).unwrap()
}

fn keys(keys: &[(u32, u32, Option<FrameSound>)]) -> Vec<Arc<Keyframe>> {
    keys.iter()
        .map(|(s, n, fs)| {
            let mut k = Keyframe::new(*s, *n);
            k.sound = fs.clone();
            Arc::new(k)
        })
        .collect()
}

/// One layer whose keyframes are `(start, duration, sound)`; 8 fps so one frame is 1000
/// output frames at 8 kHz.
fn doc_with(k: &[(u32, u32, Option<FrameSound>)], sounds: Vec<Sound>) -> Document {
    let mut d = Document::default();
    d.stage.fps = 8.0;
    let mut l = Layer::new(LayerId(1), "audio");
    l.keyframes = keys(k);
    d.timeline_mut(0).layers = vec![Arc::new(l)];
    d.sounds = sounds.into_iter().map(Arc::new).collect();
    d
}

fn with(sync: SoundSync, repeat: u32, looping: bool) -> Option<FrameSound> {
    Some(FrameSound { sound: SoundId(7), sync, repeat, looping, effect: SoundEffect::None })
}

fn effect(sync: SoundSync, effect: SoundEffect) -> Option<FrameSound> {
    Some(FrameSound { effect, ..with(sync, 1, false).unwrap() })
}

fn spans(v: &[Voice]) -> Vec<(VoiceKind, u64, u64)> {
    v.iter().map(|v| (v.kind, v.at, v.end)).collect()
}

/// The mix of output frames `0..n` at 8 kHz, as (left, right) pairs.
fn heard(v: &[Voice], n: usize) -> Vec<(f32, f32)> {
    let mut out = vec![0.0; 2 * n];
    mix(v, 8000, MixOptions::default(), 0, &mut out);
    out.chunks(2).map(|c| (c[0], c[1])).collect()
}

#[test]
fn sounds_decode_once_and_survive_a_save() {
    let s = ramp_sound(7, 2500);
    assert_eq!((s.format, s.rate, s.channels, s.frames), (SoundFormat::Wav, 8000, 1, 2500));
    assert_eq!(s.movie_frames(8.0), 3);
    assert!(s.decoded_if_ready().is_some());
    let back: Sound = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    assert!(back.decoded_if_ready().is_none(), "decoded lazily after loading");
    assert_eq!(back.decoded().unwrap().pcm.samples, s.decoded().unwrap().pcm.samples);
    assert_eq!(Sound::from_data(SoundId(1), "x", b"nope".to_vec()).err(), Some(AudioError::UnknownFormat));
    // A frame sound writes only what differs from the defaults.
    let fs = FrameSound::new(SoundId(3));
    assert_eq!(serde_json::to_string(&fs).unwrap(), r#"{"sound":3,"sync":"event"}"#);
    let full: FrameSound = serde_json::from_str(r#"{"sound":3,"sync":"stream","repeat":4,"loop":true}"#).unwrap();
    assert_eq!(full, FrameSound { sound: SoundId(3), sync: SoundSync::Stream, repeat: 4, looping: true, effect: SoundEffect::None });
}

#[test]
fn effects_are_written_by_name_and_custom_points_round_trip() {
    let mut fs = FrameSound::new(SoundId(3));
    fs.effect = SoundEffect::FadeIn;
    assert_eq!(serde_json::to_string(&fs).unwrap(), r#"{"sound":3,"sync":"event","effect":"fade_in"}"#);
    fs.effect = SoundEffect::Custom(vec![EnvelopePoint::new(0.5, 1.0, 0.25)]);
    let json = serde_json::to_string(&fs).unwrap();
    assert_eq!(json, r#"{"sound":3,"sync":"event","effect":{"custom":[{"at":0.5,"left":1.0,"right":0.25}]}}"#);
    assert_eq!(serde_json::from_str::<FrameSound>(&json).unwrap(), fs);
    for e in SoundEffect::PRESETS {
        assert_eq!(SoundEffect::from_key(e.key()), Some(e.clone()));
        let back: SoundEffect = serde_json::from_value(serde_json::json!(e.key())).unwrap();
        assert_eq!(back, e);
    }
    assert_eq!(SoundEffect::from_key("Fade In"), Some(SoundEffect::FadeIn));
    assert_eq!(SoundEffect::from_key("left"), Some(SoundEffect::LeftChannel));
    assert_eq!(SoundEffect::from_key("custom"), Some(SoundEffect::Custom(Vec::new())));
    assert_eq!(SoundEffect::from_key("louder"), None);
    // Hostile points are made safe: in range, in order, at most 16.
    let wild: Vec<EnvelopePoint> = (0..40).map(|i| EnvelopePoint::new(if i == 0 { -5.0 } else { 1e12 - f64::from(i) }, f32::NAN, 9.0)).collect();
    let clean = clean_points(&wild);
    assert_eq!(clean.len(), MAX_ENVELOPE_POINTS);
    assert_eq!((clean[0].at, clean[0].left, clean[0].right), (0.0, 0.0, 1.0));
    assert!(clean.iter().all(|p| (0.0..=MAX_ENVELOPE_SECONDS).contains(&p.at)));
    // A preset's points over a heard time are what the envelope editor starts from.
    assert_eq!(SoundEffect::FadeOut.points(2.0), vec![EnvelopePoint::new(0.0, 1.0, 1.0), EnvelopePoint::new(2.0, 0.0, 0.0)]);
    assert!(SoundEffect::None.points(2.0).is_empty());
}

#[test]
fn each_preset_shapes_the_gain_over_the_time_the_sound_is_heard() {
    // A 2500-frame event sound from frame 0: heard for 2500 output frames.
    let n = 2500u64;
    let src = |i: u64| i as f32 / 1000.0;
    type Gain = fn(f64) -> (f32, f32);
    let cases: [(SoundEffect, Gain); 6] = [
        (SoundEffect::LeftChannel, |_| (1.0, 0.0)),
        (SoundEffect::RightChannel, |_| (0.0, 1.0)),
        (SoundEffect::FadeLeftToRight, |x| (lerp(1.0, 0.0, x), lerp(0.0, 1.0, x))),
        (SoundEffect::FadeRightToLeft, |x| (lerp(0.0, 1.0, x), lerp(1.0, 0.0, x))),
        (SoundEffect::FadeIn, |x| (lerp(0.0, 1.0, x), lerp(0.0, 1.0, x))),
        (SoundEffect::FadeOut, |x| (lerp(1.0, 0.0, x), lerp(1.0, 0.0, x))),
    ];
    for (e, gain) in cases {
        let d = doc_with(&[(0, 4, effect(SoundSync::Event, e.clone()))], vec![ramp_sound(7, n as usize)]);
        let v = voices(&d, 8000);
        let out = heard(&v, n as usize);
        for i in [0, 1, 7, 625, 1250, 1999, 2499] {
            let (gl, gr) = gain(i as f64 / n as f64);
            assert_eq!(out[i as usize], (src(i) * gl, src(i) * gr), "{} at sample {i}", e.label());
        }
    }
    // None leaves the sound as it is (no envelope at all).
    let d = doc_with(&[(0, 4, with(SoundSync::Event, 1, false))], vec![ramp_sound(7, 2500)]);
    assert!(voices(&d, 8000)[0].envelope.is_none());
}

#[test]
fn fades_follow_repeats_spans_and_loops() {
    // Repeated twice: the fade runs over both plays.
    let d = doc_with(&[(0, 9, Some(FrameSound { repeat: 2, ..effect(SoundSync::Event, SoundEffect::FadeOut).unwrap() }))], vec![ramp_sound(7, 1000)]);
    let v = voices(&d, 8000);
    assert_eq!(v[0].envelope.as_ref().unwrap().points().last().unwrap().at, 2000);
    let out = heard(&v, 2000);
    assert_eq!(out[1500].0, 0.5 * lerp(1.0, 0.0, 0.75));
    // A stream fades over its span (here shorter than the sound).
    let d = doc_with(&[(1, 2, effect(SoundSync::Stream, SoundEffect::FadeIn)), (3, 1, None)], vec![ramp_sound(7, 2500)]);
    let v = voices(&d, 8000);
    assert_eq!((v[0].at, v[0].end, v[0].envelope.as_ref().unwrap().gain(1000)), (1000, 3000, (0.5, 0.5)));
    // An event loop fades over the rest of the movie.
    let d = doc_with(&[(2, 6, Some(FrameSound { looping: true, ..effect(SoundSync::Event, SoundEffect::FadeOut).unwrap() }))], vec![ramp_sound(7, 1000)]);
    let v = voices(&d, 8000);
    assert_eq!((v[0].at, v[0].end, v[0].envelope.as_ref().unwrap().points()[1].at), (2000, 8000, 6000));
}

#[test]
fn custom_points_are_seconds_from_the_sound_s_start() {
    let pts = vec![EnvelopePoint::new(0.125, 1.0, 0.0), EnvelopePoint::new(0.25, 0.5, 1.0)];
    let d = doc_with(&[(1, 4, effect(SoundSync::Event, SoundEffect::Custom(pts)))], vec![ramp_sound(7, 2500)]);
    let v = voices(&d, 8000);
    let env = v[0].envelope.as_ref().unwrap();
    assert_eq!(env.points().iter().map(|p| p.at).collect::<Vec<_>>(), vec![1000, 2000]);
    let out = heard(&v, 5000);
    // Before the first point its level holds; between, a straight line; after, the last level.
    assert_eq!(out[1000 + 500], (0.5, 0.0));
    assert_eq!(out[1000 + 1500], (1.5 * lerp(1.0, 0.5, 0.5), 1.5 * lerp(0.0, 1.0, 0.5)));
    assert_eq!(out[1000 + 2400], (2.4 * 0.5, 2.4));
    // At 16 kHz the same points land twice as far in.
    assert_eq!(voices(&d, 16_000)[0].envelope.as_ref().unwrap().points()[1].at, 4000);
}

#[test]
fn layer_volume_scales_and_mute_silences_without_changing_what_plays() {
    let mut d = doc_with(&[(0, 1, with(SoundSync::Event, 1, true)), (1, 1, with(SoundSync::Start, 1, false)), (4, 2, with(SoundSync::Stop, 1, false))], vec![ramp_sound(7, 2500)]);
    let loud = heard(&voices(&d, 8000), 6000);
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).volume = 0.5;
    let v = voices(&d, 8000);
    assert_eq!(v[0].gain, 0.5);
    let half = heard(&v, 6000);
    assert!(half.iter().zip(&loud).all(|(h, l)| h.0 == l.0 * 0.5), "volume scales every sample");
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).muted = true;
    let v = voices(&d, 8000);
    assert_eq!(spans(&v), vec![(VoiceKind::Event, 0, 4000)], "muted: the same voices, cut by the same Stop");
    assert!(heard(&v, 6000).iter().all(|&s| s == (0.0, 0.0)));
    // A muted layer's file entry; older files read as full volume, not muted.
    let json = serde_json::to_value(&*d.timeline(0).layers[0]).unwrap();
    assert_eq!((json["volume"].as_f64(), json["muted"].as_bool()), (Some(0.5), Some(true)));
    let plain = serde_json::to_value(Layer::new(LayerId(2), "x")).unwrap();
    assert!(plain.get("volume").is_none() && plain.get("muted").is_none());
    let old: Layer = serde_json::from_str(r#"{"id":2,"name":"x"}"#).unwrap();
    assert_eq!((old.volume, old.muted, old.sound_gain()), (1.0, false, 1.0));
}

/// A document whose main timeline shows, on one layer, `instances` of a symbol (`kind`,
/// `len` frames, a sound on keyframe `sound_at` of its only layer): each instance is
/// `(start, duration, loop mode, first frame, element id)` on the main timeline.
fn nested(kind: SymbolKind, len: u32, inner: &[(u32, u32, Option<FrameSound>)], instances: &[(u32, u32, LoopMode, u32, u64)], sound_len: usize) -> Document {
    let mut d = Document::default();
    d.stage.fps = 8.0;
    let mut il = Layer::new(LayerId(10), "inner");
    il.keyframes = keys(inner);
    // Pad the symbol's timeline to `len` frames with a second, silent layer.
    let mut pad = Layer::new(LayerId(11), "pad");
    pad.keyframes = vec![Arc::new(Keyframe::new(0, len))];
    d.library.push(Arc::new(Symbol { linkage: None, scale_grid_kept: None, scale_grid: None, id: SymbolId(50), name: "S".into(), kind, timeline: Timeline { layers: vec![Arc::new(il), Arc::new(pad)] } }));
    let mut l = Layer::new(LayerId(1), "outer");
    l.keyframes = instances
        .iter()
        .map(|&(s, n, loop_mode, first_frame, id)| {
            let mut k = Keyframe::new(s, n);
            k.elements.push(Arc::new(Element {
                id: ElementId(id),
                name: String::new(),
                transform: Default::default(),
                color: Default::default(),
                kind: ElementKind::Symbol { symbol: SymbolId(50), loop_mode, first_frame, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
            }));
            Arc::new(k)
        })
        .collect();
    d.timeline_mut(0).layers = vec![Arc::new(l)];
    d.sounds = vec![Arc::new(ramp_sound(7, sound_len))];
    d
}

#[test]
fn a_graphic_plays_its_sounds_on_its_own_frames_mapped_to_the_parent() {
    let ev = with(SoundSync::Event, 1, false);
    // A 3-frame graphic looping over frames 2..8: its frame 1 is entered at 3 and 6.
    let d = nested(SymbolKind::Graphic, 3, &[(0, 1, None), (1, 2, ev.clone())], &[(2, 6, LoopMode::Loop, 0, 90)], 100);
    assert_eq!(spans(&voices(&d, 8000)), vec![(VoiceKind::Event, 3000, 3100), (VoiceKind::Event, 6000, 6100)]);
    // From its frame 1: entered as it appears (2), then at 5.
    let d = nested(SymbolKind::Graphic, 3, &[(0, 1, None), (1, 2, ev.clone())], &[(2, 6, LoopMode::Loop, 1, 90)], 100);
    assert_eq!(spans(&voices(&d, 8000)).iter().map(|s| s.1).collect::<Vec<_>>(), vec![2000, 5000]);
    // Play once: frame 1 once (at 3), the last frame once (at 4) and then held, not again.
    let d = nested(SymbolKind::Graphic, 3, &[(0, 1, None), (1, 1, ev.clone()), (2, 1, ev.clone())], &[(2, 6, LoopMode::PlayOnce, 0, 90)], 100);
    assert_eq!(spans(&voices(&d, 8000)).iter().map(|s| s.1).collect::<Vec<_>>(), vec![3000, 4000]);
    // Single frame: the frame it shows is entered once.
    let d = nested(SymbolKind::Graphic, 3, &[(0, 1, None), (1, 2, ev.clone())], &[(2, 6, LoopMode::SingleFrame, 1, 90)], 100);
    assert_eq!(spans(&voices(&d, 8000)).iter().map(|s| s.1).collect::<Vec<_>>(), vec![2000]);
    // A new keyframe holding the graphic starts it again from its first frame.
    let d = nested(SymbolKind::Graphic, 3, &[(1, 2, ev)], &[(0, 2, LoopMode::Loop, 0, 90), (2, 2, LoopMode::Loop, 0, 90)], 100);
    assert_eq!(spans(&voices(&d, 8000)).iter().map(|s| s.1).collect::<Vec<_>>(), vec![1000, 3000]);
}

#[test]
fn a_stream_inside_a_graphic_follows_the_parent_playhead_joining_mid_way() {
    // A stream over the graphic's frames 0..3; the instance starts on the graphic's frame 1, so
    // the stream is heard from a frame in, until the graphic wraps (then from its start again).
    let d = nested(SymbolKind::Graphic, 3, &[(0, 3, with(SoundSync::Stream, 1, true))], &[(2, 6, LoopMode::Loop, 1, 90)], 4000);
    let v = voices(&d, 8000);
    assert_eq!(spans(&v), vec![(VoiceKind::Stream, 2000, 4000), (VoiceKind::Stream, 4000, 7000), (VoiceKind::Stream, 7000, 8000)]);
    assert_eq!((v[0].skip, v[1].skip), (1000, 0));
    let out = heard(&v, 8000);
    for (i, want) in [(2000, 1000), (3999, 2999), (4000, 0), (6999, 2999), (7000, 0)] {
        assert_eq!(out[i].0, want as f32 / 1000.0, "output frame {i} plays sound frame {want}");
    }
    // Played from the middle (the stage's playhead moved there), the stream is heard there.
    let mut mid = vec![0.0; 2];
    mix(&v, 8000, MixOptions { origin: 5000, streams_only: true }, 5000, &mut mid);
    assert_eq!(mid[0], 1.0);
    // A fade on a nested stream is shaped over its keyframe's span in the graphic.
    let d = nested(SymbolKind::Graphic, 3, &[(0, 3, effect(SoundSync::Stream, SoundEffect::FadeOut))], &[(2, 6, LoopMode::Loop, 1, 90)], 4000);
    let v = voices(&d, 8000);
    assert_eq!(v[0].envelope.as_ref().unwrap().points()[1].at, 3000);
    assert_eq!(heard(&v, 3000)[2000].0, 1.0 * lerp(1.0, 0.0, 1000.0 / 3000.0));
}

#[test]
fn a_movie_clip_plays_its_sounds_on_a_loop_from_when_it_appeared() {
    let ev = with(SoundSync::Event, 1, false);
    // A 3-frame clip held by two keyframes in a row (same instance): it carries on across them.
    let d = nested(SymbolKind::MovieClip, 3, &[(0, 1, ev.clone())], &[(1, 4, LoopMode::Loop, 0, 90), (5, 3, LoopMode::Loop, 0, 90)], 100);
    assert_eq!(spans(&voices(&d, 8000)).iter().map(|s| s.1).collect::<Vec<_>>(), vec![1000, 4000, 7000]);
    // A different instance on the second keyframe starts again.
    let d = nested(SymbolKind::MovieClip, 3, &[(0, 1, ev)], &[(1, 4, LoopMode::Loop, 0, 90), (5, 3, LoopMode::Loop, 0, 91)], 100);
    assert_eq!(spans(&voices(&d, 8000)).iter().map(|s| s.1).collect::<Vec<_>>(), vec![1000, 4000, 5000]);
    // A stream in the clip carries on across the keyframes, sample-exactly.
    let d = nested(SymbolKind::MovieClip, 8, &[(0, 8, with(SoundSync::Stream, 1, false))], &[(1, 3, LoopMode::Loop, 0, 90), (4, 4, LoopMode::Loop, 0, 90)], 8000);
    let v = voices(&d, 8000);
    assert_eq!(spans(&v), vec![(VoiceKind::Stream, 1000, 4000), (VoiceKind::Stream, 4000, 8000)]);
    let out = heard(&v, 8000);
    assert!((1000..8000).all(|i| out[i].0 == (i - 1000) as f32 / 1000.0));
}

#[test]
fn volumes_multiply_down_through_symbols_and_nesting_is_bounded() {
    let mut d = nested(SymbolKind::Graphic, 3, &[(0, 1, with(SoundSync::Event, 1, false))], &[(0, 3, LoopMode::Loop, 0, 90)], 100);
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).volume = 0.5;
    let sym = Arc::make_mut(&mut d.library[0]);
    Arc::make_mut(&mut sym.timeline.layers[0]).volume = 0.5;
    assert_eq!(voices(&d, 8000)[0].gain, 0.25);
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).muted = true;
    assert_eq!(voices(&d, 8000)[0].gain, 0.0);
    // A symbol that shows itself: followed to the depth limit, then left.
    let mut d = nested(SymbolKind::Graphic, 2, &[(0, 1, with(SoundSync::Event, 1, false))], &[(0, 2, LoopMode::Loop, 0, 90)], 10);
    let me = d.timeline(0).layers[0].clone();
    Arc::make_mut(&mut d.library[0]).timeline.layers.push(me);
    let v = voices(&d, 8000);
    assert!(!v.is_empty() && v.len() <= MAX_SYMBOL_DEPTH + 2, "{} voices", v.len());
    // A one-frame graphic looping for a day inside a long span: bounded work.
    let mut d = nested(SymbolKind::Graphic, 1, &[(0, 1, with(SoundSync::Event, 1, false))], &[(0, 24 * 3600 * 8, LoopMode::Loop, 0, 90)], 10);
    d.sounds.clear();
    let t = std::time::Instant::now();
    assert!(voices(&d, 8000).is_empty());
    assert!(t.elapsed().as_secs_f32() < 5.0, "{:?}", t.elapsed());
}

#[test]
fn the_player_s_sounds_start_stop_and_skip_like_the_layout() {
    let d = doc_with(&[(0, 1, None)], vec![ramp_sound(7, 2500)]);
    let started = |sync, looping, span| Started { sound: FrameSound { looping, ..with(sync, 1, false).unwrap() }, gain: 0.5, span };
    let mut playing = Vec::new();
    trigger(&d, &mut playing, &[started(SoundSync::Event, true, 1)], 100, 8000);
    assert_eq!((playing[0].1.at, playing[0].1.end, playing[0].1.gain), (100, u64::MAX - 100 + 100, 0.5));
    // Start is skipped while it plays; Stop cuts it; a stream lasts its span.
    trigger(&d, &mut playing, &[started(SoundSync::Start, false, 1)], 900, 8000);
    assert_eq!(playing.len(), 1);
    trigger(&d, &mut playing, &[started(SoundSync::Stop, false, 1)], 1000, 8000);
    assert_eq!(playing[0].1.end, 1000);
    trigger(&d, &mut playing, &[started(SoundSync::Stream, false, 2)], 2000, 8000);
    assert_eq!(spans(&playing.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>()), vec![(VoiceKind::Stream, 2000, 4000)], "the cut voice is dropped");
    // An endless loop's fade lasts one play.
    let mut fade = started(SoundSync::Event, true, 1);
    fade.sound.effect = SoundEffect::FadeIn;
    let mut playing = Vec::new();
    trigger(&d, &mut playing, &[fade], 0, 8000);
    assert_eq!(playing[0].1.envelope.as_ref().unwrap().points()[1].at, 2500);
}

#[test]
fn event_sounds_play_in_full_and_repeat() {
    let d = doc_with(&[(0, 2, None), (2, 1, with(SoundSync::Event, 1, false)), (3, 1, with(SoundSync::Event, 3, false))], vec![ramp_sound(7, 2500)]);
    assert_eq!(spans(&voices(&d, 8000)), vec![(VoiceKind::Event, 2000, 4500), (VoiceKind::Event, 3000, 10_500)]);
    // At 16 kHz the same sound lasts twice as many output frames.
    assert_eq!(spans(&voices(&d, 16_000))[0], (VoiceKind::Event, 4000, 9000));
}

#[test]
fn stream_sounds_end_with_their_span_and_loop_inside_it() {
    let d = doc_with(&[(1, 2, with(SoundSync::Stream, 1, false)), (3, 9, with(SoundSync::Stream, 1, true))], vec![ramp_sound(7, 2500)]);
    assert_eq!(spans(&voices(&d, 8000)), vec![(VoiceKind::Stream, 1000, 3000), (VoiceKind::Stream, 3000, 12_000)]);
}

#[test]
fn start_skips_a_playing_sound_and_stop_cuts_it() {
    let d = doc_with(&[(0, 1, with(SoundSync::Event, 1, true)), (1, 1, with(SoundSync::Start, 1, false)), (4, 2, with(SoundSync::Stop, 1, false))], vec![ramp_sound(7, 2500)]);
    assert_eq!(spans(&voices(&d, 8000)), vec![(VoiceKind::Event, 0, 4000)], "the loop is cut at frame 4; Start was skipped");
    // Start plays when nothing is playing.
    let d = doc_with(&[(0, 4, with(SoundSync::Start, 1, false))], vec![ramp_sound(7, 2500)]);
    assert_eq!(spans(&voices(&d, 8000)), vec![(VoiceKind::Event, 0, 2500)]);
}

#[test]
fn scenes_play_in_order_guides_are_silent_and_missing_sounds_are_skipped() {
    let mut d = doc_with(&[(0, 3, with(SoundSync::Event, 1, false))], vec![ramp_sound(7, 100)]);
    let first = d.scenes[0].clone();
    d.scenes.insert(0, Arc::new(Scene { name: "Intro".into(), timeline: first.timeline.clone() }));
    assert_eq!(spans(&voices(&d, 8000)), vec![(VoiceKind::Event, 0, 100), (VoiceKind::Event, 3000, 3100)]);
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).kind = LayerKind::Guide;
    assert_eq!(voices(&d, 8000).len(), 1, "guide layers are not exported, so not heard");
    d.sounds.clear();
    assert!(voices(&d, 8000).is_empty());
    assert!(has_sounds(&d));
}

#[test]
fn sounds_only_inside_symbols_count_as_sounds() {
    let d = nested(SymbolKind::Graphic, 3, &[(0, 3, with(SoundSync::Stream, 1, false))], &[(0, 3, LoopMode::Loop, 0, 90)], 100);
    assert!(!d.timeline(0).layers.iter().any(|l| l.has_sounds()));
    assert!(has_sounds(&d) && has_streams(&d));
}

#[test]
fn frame_times_follow_the_frame_rate_exactly() {
    assert_eq!(frame_start(24, 24.0, 48_000), 48_000);
    assert_eq!(frame_start(1, 29.97, 48_000), 1602);
    assert_eq!(frame_start(30_000, 30.0, 48_000), 48_000_000, "computed from the frame number: no drift");
    assert_eq!(frame_start(5, f32::NAN, 24), 5);
}
