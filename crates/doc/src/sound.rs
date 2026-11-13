//! Sounds (ADR 0020, ADR 0039): audio files in the document library, kept as the bytes they
//! were imported as and decoded once on first use, and sounds placed on keyframes,
//! Animate-style: a sound belongs to a keyframe of a layer, with a sync mode, a repeat count and
//! an effect (a volume envelope per channel).
//!
//! [`voices`] lays the whole movie's sounds out in time, those inside symbols included. It is
//! pure (time is a function): the exported soundtrack, the video's audio and stage playback all
//! come from it. [`trigger`] does the same for the interactive player (Test Movie), one frame
//! at a time.

use crate::{Document, ElementKind, Keyframe, Layer, LayerKind, LoopMode, MAX_SYMBOL_DEPTH, SymbolId, SymbolKind, Timeline, instance_start};
use anicraft_audio::{AudioError, EnvPoint, Envelope, Format, Pcm, Peaks, Voice, VoiceKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
/// Most points a custom envelope keeps.
pub const MAX_ENVELOPE_POINTS: usize = 16;
/// Latest time an envelope point may sit at, in seconds from the sound's start (a day).
pub const MAX_ENVELOPE_SECONDS: f64 = 86_400.0;

/// One volume point of a custom envelope (ADR 0039).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvelopePoint {
    /// Seconds from the sound's start (repeats included).
    pub at: f64,
    /// Volume of the left and of the right channel there, `0..=1`.
    pub left: f32,
    pub right: f32,
}

impl EnvelopePoint {
    pub fn new(at: f64, left: f32, right: f32) -> EnvelopePoint {
        EnvelopePoint { at, left, right }
    }
}

/// `points` made safe to play: at most [`MAX_ENVELOPE_POINTS`], times in
/// `0..=`[`MAX_ENVELOPE_SECONDS`], volumes in `0..=1`, in time order.
pub fn clean_points(points: &[EnvelopePoint]) -> Vec<EnvelopePoint> {
    let level = |g: f32| if g.is_finite() { g.clamp(0.0, 1.0) } else { 0.0 };
    let mut out: Vec<EnvelopePoint> = points
        .iter()
        .take(MAX_ENVELOPE_POINTS)
        .map(|p| EnvelopePoint { at: if p.at.is_finite() { p.at.clamp(0.0, MAX_ENVELOPE_SECONDS) } else { 0.0 }, left: level(p.left), right: level(p.right) })
        .collect();
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    out
}

/// What a frame sound's volume does over its time, as Animate's Effect menu (ADR 0039): the
/// presets are shaped over the time the sound is heard (one play times its repeats, its span
/// for a stream, the rest of the movie for an event loop); Custom holds points of its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundEffect {
    #[default]
    None,
    LeftChannel,
    RightChannel,
    FadeLeftToRight,
    FadeRightToLeft,
    FadeIn,
    FadeOut,
    /// Volume points of its own (Edit Envelope), times from the sound's start.
    Custom(Vec<EnvelopePoint>),
}

impl SoundEffect {
    /// The presets, in menu order (Custom comes after them).
    pub const PRESETS: [SoundEffect; 7] = [
        SoundEffect::None,
        SoundEffect::LeftChannel,
        SoundEffect::RightChannel,
        SoundEffect::FadeLeftToRight,
        SoundEffect::FadeRightToLeft,
        SoundEffect::FadeIn,
        SoundEffect::FadeOut,
    ];

    pub fn key(&self) -> &'static str {
        match self {
            SoundEffect::None => "none",
            SoundEffect::LeftChannel => "left_channel",
            SoundEffect::RightChannel => "right_channel",
            SoundEffect::FadeLeftToRight => "fade_left_to_right",
            SoundEffect::FadeRightToLeft => "fade_right_to_left",
            SoundEffect::FadeIn => "fade_in",
            SoundEffect::FadeOut => "fade_out",
            SoundEffect::Custom(_) => "custom",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SoundEffect::None => "None",
            SoundEffect::LeftChannel => "Left channel",
            SoundEffect::RightChannel => "Right channel",
            SoundEffect::FadeLeftToRight => "Fade left to right",
            SoundEffect::FadeRightToLeft => "Fade right to left",
            SoundEffect::FadeIn => "Fade in",
            SoundEffect::FadeOut => "Fade out",
            SoundEffect::Custom(_) => "Custom",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self {
            SoundEffect::None => "Full volume on both channels",
            SoundEffect::LeftChannel => "Plays on the left channel only",
            SoundEffect::RightChannel => "Plays on the right channel only",
            SoundEffect::FadeLeftToRight => "Moves from the left channel to the right while it plays",
            SoundEffect::FadeRightToLeft => "Moves from the right channel to the left while it plays",
            SoundEffect::FadeIn => "Rises from silence to full volume while it plays",
            SoundEffect::FadeOut => "Falls from full volume to silence while it plays",
            SoundEffect::Custom(_) => "Volume points of your own, per channel (Edit Envelope)",
        }
    }

    /// The effect a key names (`custom` with no points yet), forgiving case, spaces, dashes.
    pub fn from_key(s: &str) -> Option<SoundEffect> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        if s == "custom" {
            return Some(SoundEffect::Custom(Vec::new()));
        }
        SoundEffect::PRESETS.into_iter().find(|e| e.key() == s || e.key().replace("_channel", "") == s)
    }

    pub fn is_none(&self) -> bool {
        *self == SoundEffect::None
    }

    /// A preset's points: (at the end of the heard time?, left, right).
    fn shape(&self) -> &'static [(bool, f32, f32)] {
        match self {
            SoundEffect::None | SoundEffect::Custom(_) => &[],
            SoundEffect::LeftChannel => &[(false, 1.0, 0.0)],
            SoundEffect::RightChannel => &[(false, 0.0, 1.0)],
            SoundEffect::FadeLeftToRight => &[(false, 1.0, 0.0), (true, 0.0, 1.0)],
            SoundEffect::FadeRightToLeft => &[(false, 0.0, 1.0), (true, 1.0, 0.0)],
            SoundEffect::FadeIn => &[(false, 0.0, 0.0), (true, 1.0, 1.0)],
            SoundEffect::FadeOut => &[(false, 1.0, 1.0), (true, 0.0, 0.0)],
        }
    }

    /// Its points over a sound heard for `seconds` (cleaned); empty for None (full volume).
    /// What the envelope editor starts from when a preset is edited.
    pub fn points(&self, seconds: f64) -> Vec<EnvelopePoint> {
        match self {
            SoundEffect::Custom(p) => clean_points(p),
            preset => {
                let end = if seconds.is_finite() { seconds.clamp(0.0, MAX_ENVELOPE_SECONDS) } else { 0.0 };
                preset.shape().iter().map(|&(at_end, left, right)| EnvelopePoint { at: if at_end { end } else { 0.0 }, left, right }).collect()
            }
        }
    }

    /// The envelope at output rate `rate` for a sound faded over `length` output frames (the
    /// time it is heard); `None` at full volume.
    pub fn envelope(&self, length: u64, rate: u32) -> Option<Arc<Envelope>> {
        let points: Vec<EnvPoint> = match self {
            SoundEffect::None => return None,
            SoundEffect::Custom(p) => clean_points(p).into_iter().map(|p| EnvPoint { at: (p.at * f64::from(rate)).round() as u64, left: p.left, right: p.right }).collect(),
            preset => preset.shape().iter().map(|&(at_end, left, right)| EnvPoint { at: if at_end { length } else { 0 }, left, right }).collect(),
        };
        Envelope::new(points).map(Arc::new)
    }
}

/// A sound placed on a keyframe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Its volume envelope per channel (ADR 0039).
    #[serde(default, skip_serializing_if = "SoundEffect::is_none")]
    pub effect: SoundEffect,
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
        FrameSound { sound, sync: SoundSync::Event, repeat: 1, looping: false, effect: SoundEffect::None }
    }

    /// How long it is heard from its start when nothing stops it, in output frames: `once` is
    /// one play, `span` its keyframe's span and `rest` (for an event loop) the time left in the
    /// movie.
    pub fn heard(&self, once: u64, span: u64, rest: u64) -> u64 {
        let repeats = once.saturating_mul(u64::from(self.repeat.clamp(1, MAX_REPEAT)));
        match (self.sync, self.looping) {
            (SoundSync::Stream, true) => span,
            (SoundSync::Stream, false) => repeats.min(span),
            (_, true) => rest,
            (_, false) => repeats,
        }
    }

    /// [`FrameSound::heard`] in seconds, for a keyframe `span` frames long with `rest` frames
    /// from it to the end of its timeline, at `fps`: the time its effect is shaped over (what
    /// the envelope editor shows).
    pub fn heard_seconds(&self, sound: &Sound, span: u32, rest: u32, fps: f32) -> f64 {
        let rate = sound.rate.max(1);
        let frames = self.heard(sound.frames, frame_start(u64::from(span), fps, rate), frame_start(u64::from(rest), fps, rate));
        frames as f64 / f64::from(rate)
    }
}

/// Output frame (at `rate`) where movie frame `frame` starts, at `fps`.
pub fn frame_start(frame: u64, fps: f32, rate: u32) -> u64 {
    let fps = if fps.is_finite() && fps > 0.0 { f64::from(fps) } else { 24.0 };
    (frame as f64 * f64::from(rate) / fps).round() as u64
}

/// The voice of `fs` heard from output frame `at`, `skip` frames into its time, for `heard`
/// frames from its start (no further than `limit`), its effect shaped over `fade` frames.
#[allow(clippy::too_many_arguments)]
fn voice(pcm: &Arc<Pcm>, fs: &FrameSound, at: u64, skip: u64, heard: u64, fade: u64, limit: u64, gain: f32, rate: u32) -> Option<Voice> {
    let kind = if fs.sync == SoundSync::Stream { VoiceKind::Stream } else { VoiceKind::Event };
    let end = at.saturating_add(heard.saturating_sub(skip)).min(limit);
    (end > at).then(|| Voice { pcm: pcm.clone(), kind, at, end, gain, skip, envelope: fs.effect.envelope(fade, rate) })
}

/// Most timeline stretches the layout follows (a stretch is a symbol's timeline shown over a
/// run of frames): a one-frame graphic looping for hours inside another goes quiet past this
/// instead of taking the app's time.
pub const MAX_STRETCHES: usize = 100_000;

/// A stretch of a timeline in movie time: movie frames `g0 ..` show the timeline's frames
/// `l0 ..` one per frame (`advancing`), or hold its frame `l0` for `n` frames. `enters`: frame
/// `l0` is entered at `g0` (not carried on from just before), so its event sounds start.
#[derive(Clone, Copy, Debug)]
struct Stretch {
    g0: u64,
    l0: u64,
    n: u64,
    advancing: bool,
    enters: bool,
    gain: f32,
}

/// A frame sound reached by the layout.
struct Placed {
    /// Output frame where it is first heard, and how far into its time that is.
    at: u64,
    skip: u64,
    /// Where its stretch ends it (streams; `u64::MAX` for the others).
    limit: u64,
    /// Its keyframe's span, in output frames.
    span: u64,
    fs: FrameSound,
    gain: f32,
}

struct Layout<'a> {
    doc: &'a Document,
    fps: f32,
    rate: u32,
    /// Symbols with sounds on their timelines, or inside the symbols they show.
    sounding: HashSet<SymbolId>,
    placed: Vec<Placed>,
    budget: usize,
}

/// The symbols whose timelines hold sounds, directly or through the symbols on them.
fn sounding_symbols(doc: &Document) -> HashSet<SymbolId> {
    let mut shown_in: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
    let mut set = HashSet::new();
    let mut queue = Vec::new();
    for sym in &doc.library {
        for k in sym.timeline.layers.iter().flat_map(|l| l.keyframes.iter()) {
            if k.sound.is_some() && set.insert(sym.id) {
                queue.push(sym.id);
            }
            for el in crate::expand_groups(&k.elements).iter() {
                if let ElementKind::Symbol { symbol, .. } = &el.kind {
                    shown_in.entry(*symbol).or_default().push(sym.id);
                }
            }
        }
    }
    while let Some(id) = queue.pop() {
        for outer in shown_in.get(&id).into_iter().flatten() {
            if set.insert(*outer) {
                queue.push(*outer);
            }
        }
    }
    set
}

fn frame32(f: u64) -> u32 {
    u32::try_from(f).unwrap_or(u32::MAX)
}

impl<'a> Layout<'a> {
    fn out(&self, movie_frame: u64) -> u64 {
        frame_start(movie_frame, self.fps, self.rate)
    }

    /// The sounds of `tl` over a stretch, and those of the symbols shown on it.
    fn timeline(&mut self, tl: &'a Timeline, st: Stretch, depth: usize) {
        if depth > MAX_SYMBOL_DEPTH || self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let hi = if st.advancing { st.l0.saturating_add(st.n) } else { st.l0.saturating_add(1) };
        for layer in tl.layers.iter().filter(|l| !matches!(l.kind, LayerKind::Guide | LayerKind::Folder | LayerKind::Camera)) {
            let gain = st.gain * layer.sound_gain();
            let first = layer.keyframes.partition_point(|k| u64::from(k.end()) <= st.l0);
            for k in layer.keyframes.iter().skip(first).take_while(|k| u64::from(k.start) < hi) {
                if let Some(fs) = &k.sound {
                    self.sound(fs, k, st, gain);
                }
                // (A symbol inside a group plays as if it were on the group's layer, ADR 0073.)
                for el in crate::expand_groups(&k.elements).iter() {
                    if matches!(&el.kind, ElementKind::Symbol { symbol, .. } if self.sounding.contains(symbol)) {
                        self.instance(layer, k, el, st, gain, depth);
                    }
                }
            }
        }
    }

    /// Keyframe `k`'s sound, if the stretch reaches it: an event (or start, or stop) sound when
    /// its keyframe is entered; a stream over the part of its span the stretch plays, joined
    /// mid-way when the stretch starts inside it.
    fn sound(&mut self, fs: &FrameSound, k: &Keyframe, st: Stretch, gain: f32) {
        let (start, end) = (u64::from(k.start), u64::from(k.end()));
        let span = frame_start(end - start, self.fps, self.rate);
        if fs.sync == SoundSync::Stream {
            let (a, b) = (start.max(st.l0), end.min(st.l0.saturating_add(st.n)));
            if !st.advancing || a >= b {
                return;
            }
            let (at, limit) = (self.out(st.g0 + (a - st.l0)), self.out(st.g0 + (b - st.l0)));
            let skip = frame_start(a - start, self.fps, self.rate);
            self.placed.push(Placed { at, skip, limit, span, fs: fs.clone(), gain });
            return;
        }
        let entered = if st.advancing { start >= st.l0 && start < st.l0.saturating_add(st.n) && (start > st.l0 || st.enters) } else { start == st.l0 && st.enters };
        if entered {
            let at = self.out(st.g0 + (start - st.l0));
            self.placed.push(Placed { at, skip: 0, limit: u64::MAX, span, fs: fs.clone(), gain });
        }
    }

    /// A symbol instance on keyframe `k` of `layer`: its timeline over the frames it shows. A
    /// graphic keeps step with this timeline (loop mode and first frame apply); a movie clip
    /// plays its own on a loop from when the instance appeared, as the stage shows it.
    #[allow(clippy::too_many_arguments)]
    fn instance(&mut self, layer: &Layer, k: &Keyframe, el: &crate::Element, st: Stretch, gain: f32, depth: usize) {
        let ElementKind::Symbol { symbol, loop_mode, first_frame, .. } = &el.kind else { return };
        let Some(sym) = self.doc.symbol(*symbol) else { return };
        let tl = &sym.timeline;
        let len = u64::from(tl.frame_count().max(1));
        let first = u64::from(*first_frame);
        let start = u64::from(k.start);
        let child = |l0: u64, n: u64, advancing: bool, enters: bool, g0: u64| Stretch { g0, l0, n, advancing, enters, gain };
        if !st.advancing {
            // The parent holds a frame: the instance holds the one it shows there.
            let p = st.l0;
            let lf = match sym.kind {
                SymbolKind::Graphic => u64::from(loop_mode.local_frame(frame32(p.saturating_sub(start)), *first_frame, frame32(len))),
                SymbolKind::MovieClip => p.saturating_sub(u64::from(instance_start(layer, frame32(p), el.id, *symbol))) % len,
                SymbolKind::Button => return,
            };
            self.timeline(tl, child(lf, st.n, false, st.enters, st.g0), depth + 1);
            return;
        }
        let (pa, pb) = (start.max(st.l0), u64::from(k.end()).min(st.l0.saturating_add(st.n)));
        if pa >= pb {
            return;
        }
        let (g, n) = (st.g0 + (pa - st.l0), pb - pa);
        let enters = pa > st.l0 || st.enters;
        match (sym.kind, loop_mode) {
            (SymbolKind::Graphic, LoopMode::Loop) => self.looped(tl, g, (first + (pa - start)) % len, n, len, enters, gain, depth),
            (SymbolKind::Graphic, LoopMode::PlayOnce) => {
                // Plays to its last frame, then holds it.
                let lf = (first + (pa - start)).min(len - 1);
                let run = (len - lf).min(n);
                self.timeline(tl, child(lf, run, true, enters, g), depth + 1);
                if n > run {
                    self.timeline(tl, child(len - 1, n - run, false, false, g + run), depth + 1);
                }
            }
            (SymbolKind::Graphic, LoopMode::SingleFrame) => self.timeline(tl, child(first.min(len - 1), n, false, enters, g), depth + 1),
            (SymbolKind::MovieClip, _) => {
                // Carried on across keyframes that hold the same instance: entered only where it
                // appeared (or where this stretch jumps in).
                let s0 = u64::from(instance_start(layer, frame32(pa), el.id, *symbol)).min(pa);
                let enters = pa == s0 || (pa == st.l0 && st.enters);
                self.looped(tl, g, (pa - s0) % len, n, len, enters, gain, depth);
            }
            // Buttons sound only under the pointer, in the interactive player (ADR 0057).
            (SymbolKind::Button, _) => {}
        }
    }

    /// A timeline `len` frames long played from its frame `lf` for `n` frames, round and round.
    #[allow(clippy::too_many_arguments)]
    fn looped(&mut self, tl: &'a Timeline, mut g: u64, mut lf: u64, mut n: u64, len: u64, mut enters: bool, gain: f32, depth: usize) {
        if depth >= MAX_SYMBOL_DEPTH {
            return;
        }
        while n > 0 && self.budget > 0 {
            let run = len.saturating_sub(lf).clamp(1, n);
            self.timeline(tl, Stretch { g0: g, l0: lf, n: run, advancing: true, enters, gain }, depth + 1);
            self.budget = self.budget.saturating_sub(1);
            (g, n, lf, enters) = (g + run, n - run, 0, true);
        }
    }
}

/// Every sound of the movie (all scenes in order, every layer but guides, and the sounds inside
/// the symbols on them) laid out in output frames at `rate`, in the order they start. Sync
/// modes are resolved by playing the timeline from the start: Start skips a sound already
/// playing, Stop cuts it, Stream ends with its span. A graphic's sounds play on its own frames
/// as this timeline shows them; a movie clip's as the stage plays it, on a loop from when the
/// instance appeared. Layer volumes multiply down through the symbols, a muted layer is silent.
/// Missing or undecodable sounds are left out. Pure: same document, same voices.
pub fn voices(doc: &Document, rate: u32) -> Vec<Voice> {
    let fps = doc.stage.fps;
    let movie_end = frame_start(u64::from(doc.frame_count()), fps, rate);
    let mut lay = Layout { doc, fps, rate, sounding: sounding_symbols(doc), placed: Vec::new(), budget: MAX_STRETCHES };
    for (si, scene) in doc.scenes.iter().enumerate() {
        let st = Stretch { g0: u64::from(doc.scene_start(si)), l0: 0, n: u64::from(scene.timeline.frame_count()), advancing: true, enters: true, gain: 1.0 };
        lay.timeline(&scene.timeline, st, 0);
    }
    let mut placed = lay.placed;
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
        let heard = p.fs.heard(Voice::play_frames(&dec.pcm, rate), p.span, movie_end.saturating_sub(p.at));
        if let Some(v) = voice(&dec.pcm, &p.fs, p.at, p.skip, heard, heard, p.limit, p.gain, rate) {
            out.push((sound, v));
        }
    }
    out.into_iter().map(|(_, v)| v).collect()
}

/// A sound the interactive player reached: the keyframe it entered held it (ADR 0039).
#[derive(Clone, Debug, PartialEq)]
pub struct Started {
    pub sound: FrameSound,
    /// The volume of its layer times those of the layers holding the symbols it is in.
    pub gain: f32,
    /// Its keyframe's span, in frames.
    pub span: u32,
}

/// Starts what one step of the interactive player reached, at output frame `at` (`rate`),
/// among the voices `playing` (with their sounds' ids; those that have ended are dropped first).
/// Event, Start and Stop work as in [`voices`]; a stream plays over its keyframe's span. The
/// player's movie goes round for as long as it runs, so an event loop has no end: its effect
/// is shaped over one play.
pub fn trigger(doc: &Document, playing: &mut Vec<(SoundId, Voice)>, started: &[Started], at: u64, rate: u32) {
    playing.retain(|(_, v)| v.end > at);
    for s in started {
        let id = s.sound.sound;
        match s.sound.sync {
            SoundSync::Stop => {
                for (_, v) in playing.iter_mut().filter(|(i, v)| *i == id && v.at <= at) {
                    v.end = at.max(v.at);
                }
                continue;
            }
            SoundSync::Start if playing.iter().any(|(i, v)| *i == id && v.at <= at && at < v.end) => continue,
            _ => {}
        }
        let Some(dec) = doc.sound(id).and_then(Sound::decoded) else { continue };
        let once = Voice::play_frames(&dec.pcm, rate);
        let span = frame_start(u64::from(s.span), doc.stage.fps, rate);
        let endless = s.sound.sync != SoundSync::Stream && s.sound.looping;
        let heard = s.sound.heard(once, span, u64::MAX - at);
        if let Some(v) = voice(&dec.pcm, &s.sound, at, 0, heard, if endless { once } else { heard }, u64::MAX, s.gain, rate) {
            playing.push((id, v));
        }
    }
}

/// Whether any keyframe in the document (scenes and symbols) has a sound.
pub fn has_sounds(doc: &Document) -> bool {
    doc.timelines().any(|tl| tl.layers.iter().any(|l| l.has_sounds()))
}

/// Whether any keyframe in the document has a stream sound (what scrubbing plays).
pub fn has_streams(doc: &Document) -> bool {
    doc.timelines().any(|tl| tl.layers.iter().any(|l| l.keyframes.iter().any(|k| k.sound.as_ref().is_some_and(|f| f.sync == SoundSync::Stream))))
}

#[cfg(test)]
#[path = "sound_tests.rs"]
mod tests;
