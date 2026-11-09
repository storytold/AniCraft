//! Sounds (ADR 0020): audio files in the document library, kept as the bytes they were
//! imported as and decoded once on first use, and sounds placed on keyframes, Animate-style:
//! a sound belongs to a keyframe of a layer, with a sync mode and a repeat count.
//!
//! [`voices`] lays the whole movie's sounds out in time. It is pure (time is a function): the
//! exported soundtrack, the video's audio and stage playback all come from it.

use crate::Document;
use anicraft_audio::{AudioError, Format, Pcm, Peaks, Voice, VoiceKind};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// Unique id of a sound in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SoundId(pub u64);

/// How a sound's data is encoded (what was imported; kept as it came).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundFormat {
    Wav,
    Ogg,
    Mp3,
}

impl SoundFormat {
    pub fn label(self) -> &'static str {
        match self {
            SoundFormat::Wav => "WAV",
            SoundFormat::Ogg => "Ogg Vorbis",
            SoundFormat::Mp3 => "MP3",
        }
    }
}

/// A decoded sound and its waveform summary.
#[derive(Debug)]
pub struct Decoded {
    pub pcm: Arc<Pcm>,
    pub peaks: Peaks,
}

/// A sound in the library.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sound {
    pub id: SoundId,
    pub name: String,
    pub format: SoundFormat,
    /// Sample rate (Hz), channels (1 or 2) and length in frames, as decoded.
    pub rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// The file as imported.
    #[serde(with = "crate::bitmap::base64_bytes")]
    pub data: Arc<Vec<u8>>,
    #[serde(skip)]
    decoded: Arc<OnceLock<Option<Arc<Decoded>>>>,
}

impl PartialEq for Sound {
    fn eq(&self, o: &Sound) -> bool {
        self.id == o.id
            && self.name == o.name
            && self.format == o.format
            && self.rate == o.rate
            && self.channels == o.channels
            && self.frames == o.frames
            && (Arc::ptr_eq(&self.data, &o.data) || self.data == o.data)
    }
}

impl Sound {
    /// A sound from a WAV, Ogg Vorbis or MP3 file's bytes (told apart by content). Decoded now,
    /// so a file that can't be played is refused at import.
    pub fn from_data(id: SoundId, name: impl Into<String>, data: Vec<u8>) -> Result<Sound, AudioError> {
        let format = match Format::sniff(&data).ok_or(AudioError::UnknownFormat)? {
            Format::Wav => SoundFormat::Wav,
            Format::Vorbis => SoundFormat::Ogg,
            Format::Mp3 => SoundFormat::Mp3,
        };
        let pcm = anicraft_audio::decode(&data)?;
        let (rate, channels, frames) = (pcm.rate, pcm.channels, pcm.frames() as u64);
        let decoded = OnceLock::new();
        let _ = decoded.set(Some(Arc::new(Decoded { peaks: Peaks::of(&pcm), pcm: Arc::new(pcm) })));
        Ok(Sound { id, name: name.into(), format, rate, channels, frames, data: Arc::new(data), decoded: Arc::new(decoded) })
    }

    /// The decoded sound, decoding it on first use (shared by every clone). `None` when the data
    /// doesn't decode.
    pub fn decoded(&self) -> Option<Arc<Decoded>> {
        self.decoded
            .get_or_init(|| {
                let pcm = anicraft_audio::decode(&self.data).ok()?;
                Some(Arc::new(Decoded { peaks: Peaks::of(&pcm), pcm: Arc::new(pcm) }))
            })
            .clone()
    }

    /// The decoded sound if decoding has finished; never waits (for drawing).
    pub fn decoded_if_ready(&self) -> Option<Arc<Decoded>> {
        self.decoded.get().cloned().flatten()
    }

    /// Length in seconds.
    pub fn seconds(&self) -> f64 {
        self.frames as f64 / f64::from(self.rate.max(1))
    }

    /// Length in movie frames at `fps` (rounded up, at least 1).
    pub fn movie_frames(&self, fps: f32) -> u32 {
        let fps = if fps.is_finite() && fps > 0.0 { f64::from(fps) } else { 24.0 };
        ((self.seconds() * fps).ceil() as u32).max(1)
    }
}

/// When a keyframe's sound plays, as in Animate's Sync menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundSync {
    /// Plays in full from its keyframe, whatever the timeline does.
    #[default]
    Event,
    /// Like Event, unless the same sound is already playing.
    Start,
    /// Stops the sound wherever it is playing.
    Stop,
    /// Tied to the frames: plays over its keyframe's span only, and follows the playhead.
    Stream,
}

impl SoundSync {
    pub const ALL: [SoundSync; 4] = [SoundSync::Event, SoundSync::Start, SoundSync::Stop, SoundSync::Stream];

    pub fn key(self) -> &'static str {
        match self {
            SoundSync::Event => "event",
            SoundSync::Start => "start",
            SoundSync::Stop => "stop",
            SoundSync::Stream => "stream",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SoundSync::Event => "Event",
            SoundSync::Start => "Start",
            SoundSync::Stop => "Stop",
            SoundSync::Stream => "Stream",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SoundSync::Event => "Plays in full from this keyframe, independent of the timeline",
            SoundSync::Start => "Like Event, but not if the sound is already playing",
            SoundSync::Stop => "Stops this sound",
            SoundSync::Stream => "Tied to the frames: plays over this span and follows the playhead (lip sync)",
        }
    }

    pub fn from_key(s: &str) -> Option<SoundSync> {
        let s = s.trim().to_ascii_lowercase();
        SoundSync::ALL.into_iter().find(|m| m.key() == s)
    }
}

/// Most repeats a frame sound takes.
pub const MAX_REPEAT: u32 = 9_999;

/// A sound placed on a keyframe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameSound {
    pub sound: SoundId,
    #[serde(default)]
    pub sync: SoundSync,
    /// Times it plays in a row (1 = once); `looping` overrides it.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub repeat: u32,
    /// Plays again and again: to the movie's end (event) or the span's end (stream).
    #[serde(default, rename = "loop", skip_serializing_if = "is_false")]
    pub looping: bool,
}

fn one() -> u32 {
    1
}
fn is_one(n: &u32) -> bool {
    *n == 1
}
fn is_false(b: &bool) -> bool {
    !*b
}

impl FrameSound {
    pub fn new(sound: SoundId) -> FrameSound {
        FrameSound { sound, sync: SoundSync::Event, repeat: 1, looping: false }
    }
}

/// Output frame (at `rate`) where movie frame `frame` starts, at `fps`.
pub fn frame_start(frame: u64, fps: f32, rate: u32) -> u64 {
    let fps = if fps.is_finite() && fps > 0.0 { f64::from(fps) } else { 24.0 };
    (frame as f64 * f64::from(rate) / fps).round() as u64
}

/// Every sound of the movie (all scenes in order, every layer but guides) laid out in output
/// frames at `rate`, in the order they start. Sync modes are resolved by playing the timeline
/// from the start: Start skips a sound already playing, Stop cuts it, Stream ends with its
/// span. Missing or undecodable sounds are left out. Pure: same document, same voices.
pub fn voices(doc: &Document, rate: u32) -> Vec<Voice> {
    struct Placed {
        at: u64,
        span_end: u64,
        fs: FrameSound,
    }
    let fps = doc.stage.fps;
    let movie_end = frame_start(u64::from(doc.frame_count()), fps, rate);
    let mut placed = Vec::new();
    for (si, scene) in doc.scenes.iter().enumerate() {
        let base = u64::from(doc.scene_start(si));
        for layer in scene.timeline.layers.iter().filter(|l| l.kind != crate::LayerKind::Guide) {
            for k in &layer.keyframes {
                if let Some(fs) = k.sound {
                    let at = frame_start(base + u64::from(k.start), fps, rate);
                    placed.push(Placed { at, span_end: frame_start(base + u64::from(k.end()), fps, rate), fs });
                }
            }
        }
    }
    // Stable: sounds starting together keep scene, layer and keyframe order.
    placed.sort_by_key(|p| p.at);
    let mut out: Vec<(SoundId, Voice)> = Vec::new();
    for p in placed {
        let sound = p.fs.sound;
        let sounding = |v: &Voice| v.at <= p.at && p.at < v.end;
        match p.fs.sync {
            SoundSync::Stop => {
                for (id, v) in &mut out {
                    if *id == sound && sounding(v) {
                        v.end = p.at;
                    }
                }
                continue;
            }
            SoundSync::Start if out.iter().any(|(id, v)| *id == sound && sounding(v)) => continue,
            _ => {}
        }
        let Some(dec) = doc.sound(sound).and_then(Sound::decoded) else { continue };
        let once = Voice::play_frames(&dec.pcm, rate);
        let repeats = once.saturating_mul(u64::from(p.fs.repeat.clamp(1, MAX_REPEAT)));
        let (kind, end) = match p.fs.sync {
            SoundSync::Stream => (VoiceKind::Stream, if p.fs.looping { p.span_end } else { p.at.saturating_add(repeats).min(p.span_end) }),
            _ => (VoiceKind::Event, if p.fs.looping { movie_end.max(p.at) } else { p.at.saturating_add(repeats) }),
        };
        if end > p.at {
            out.push((sound, Voice { pcm: dec.pcm.clone(), kind, at: p.at, end, gain: 1.0 }));
        }
    }
    out.into_iter().map(|(_, v)| v).collect()
}

/// Whether any keyframe in the movie has a sound.
pub fn has_sounds(doc: &Document) -> bool {
    doc.scenes.iter().any(|s| s.timeline.layers.iter().any(|l| l.keyframes.iter().any(|k| k.sound.is_some())))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{Keyframe, Layer, LayerId, LayerKind, Scene};

    /// A sound of `n` frames at 8 kHz whose sample `i` is `i / 1000`.
    fn ramp_sound(id: u64, n: usize) -> Sound {
        let s: Vec<f32> = (0..n).map(|i| i as f32 / 1000.0).collect();
        Sound::from_data(SoundId(id), "ramp.wav", anicraft_audio::wav::encode(8000, 1, &s, anicraft_audio::wav::Bits::Float32)).unwrap()
    }

    /// One layer whose keyframes are `(start, duration, sound)`; 8 fps so one frame is 1000
    /// output frames at 8 kHz.
    fn doc_with(keys: &[(u32, u32, Option<FrameSound>)], sounds: Vec<Sound>) -> Document {
        let mut d = Document::default();
        d.stage.fps = 8.0;
        let mut l = Layer::new(LayerId(1), "audio");
        l.keyframes = keys
            .iter()
            .map(|&(s, n, fs)| {
                let mut k = Keyframe::new(s, n);
                k.sound = fs;
                Arc::new(k)
            })
            .collect();
        d.timeline_mut(0).layers = vec![Arc::new(l)];
        d.sounds = sounds.into_iter().map(Arc::new).collect();
        d
    }

    fn with(sync: SoundSync, repeat: u32, looping: bool) -> Option<FrameSound> {
        Some(FrameSound { sound: SoundId(7), sync, repeat, looping })
    }

    fn spans(v: &[Voice]) -> Vec<(VoiceKind, u64, u64)> {
        v.iter().map(|v| (v.kind, v.at, v.end)).collect()
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
        assert_eq!(full, FrameSound { sound: SoundId(3), sync: SoundSync::Stream, repeat: 4, looping: true });
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
        d.scenes.insert(0, Arc::new(Scene { name: "Intro".into(), timeline: first.timeline.clone(), ..Scene::new("Intro") }));
        assert_eq!(spans(&voices(&d, 8000)), vec![(VoiceKind::Event, 0, 100), (VoiceKind::Event, 3000, 3100)]);
        Arc::make_mut(&mut d.timeline_mut(0).layers[0]).kind = LayerKind::Guide;
        assert_eq!(voices(&d, 8000).len(), 1, "guide layers are not exported, so not heard");
        d.sounds.clear();
        assert!(voices(&d, 8000).is_empty());
        assert!(has_sounds(&d));
    }

    #[test]
    fn frame_times_follow_the_frame_rate_exactly() {
        assert_eq!(frame_start(24, 24.0, 48_000), 48_000);
        assert_eq!(frame_start(1, 29.97, 48_000), 1602);
        assert_eq!(frame_start(30_000, 30.0, 48_000), 48_000_000, "computed from the frame number: no drift");
        assert_eq!(frame_start(5, f32::NAN, 24), 5);
    }
}
