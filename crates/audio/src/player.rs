//! The device-independent half of playback. The shell's audio callback calls
//! [`Player::fill`]; the UI tells the player where to play from, and reads back how far it has
//! got (the audio clock drives the stage during playback, so picture follows sound).
//!
//! What it plays is [`mix`](crate::mix) of the current voices, so playback and export agree
//! sample for sample, apart from short ramps at starts and stops that keep them click-free, and
//! the monitor volume (ADR 0095), which is the listener's and never reaches an export.
//!
//! **Event sounds play to their end** (ADR 0095): when looped playback jumps back, or the
//! playhead stops on the last frame, the event sounds still sounding carry on as *tails* on a
//! clock of their own, as Animate plays them. Only [`Player::stop`] (the user stopping
//! playback) silences them.

use crate::mix::{MixOptions, Voice, VoiceKind, mix};
use std::sync::Arc;

/// Length of the fade at a start or a stop (about 5 ms at 48 kHz).
pub const RAMP_FRAMES: u64 = 256;

/// The most event sounds carried on at once after a jump (the oldest go first).
pub const MAX_TAILS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Idle,
    Playing,
    /// A short snippet while scrubbing, ending at `until`.
    Snippet { until: u64 },
}

pub struct Player {
    voices: Arc<Vec<Voice>>,
    rate: u32,
    mode: Mode,
    opts: MixOptions,
    pos: u64,
    started: u64,
    /// Frames left of the fade-out after a stop.
    stopping: Option<u64>,
    /// Event sounds carried on after a jump or the end, on their own clock (`tail_pos`).
    tails: Vec<Voice>,
    tail_pos: u64,
    /// The monitor volume asked for, and the one the last frame was played at (it glides to
    /// the new value over [`RAMP_FRAMES`], so moving the slider never clicks).
    volume: f32,
    gain_now: f32,
    /// Frames in the last buffer filled, and how far behind the device plays what it is given
    /// (output frames): the frame heard is behind [`Player::position`] by these.
    last_fill: u64,
    latency: u64,
    scratch: Vec<f32>,
    tail_scratch: Vec<f32>,
}

impl Player {
    /// A silent player for output at `rate`.
    pub fn new(rate: u32) -> Player {
        Player {
            voices: Arc::new(Vec::new()),
            rate: rate.max(1),
            mode: Mode::Idle,
            opts: MixOptions::default(),
            pos: 0,
            started: 0,
            stopping: None,
            tails: Vec::new(),
            tail_pos: 0,
            volume: 1.0,
            gain_now: 1.0,
            last_fill: 0,
            latency: 0,
            scratch: Vec::new(),
            tail_scratch: Vec::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// What there is to hear (laid out for this player's rate). Playback carries on from the
    /// same position, so an edit during playback is heard at once.
    pub fn set_voices(&mut self, voices: Arc<Vec<Voice>>) {
        self.voices = voices;
    }

    /// The monitor volume, `0..=1` (`0` is mute; non-finite values are mute). Playback only:
    /// exports never hear it.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 0.0 };
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Plays from output frame `from`: stream sounds from there, event sounds as their
    /// keyframes are reached. Event sounds already playing are cut (a new start).
    pub fn play(&mut self, from: u64) {
        self.tails.clear();
        self.start(Mode::Playing, from, false);
    }

    /// Jumps playback to `from` (looped playback going round), letting the event sounds that
    /// are sounding play to their end.
    pub fn wrap(&mut self, from: u64) {
        self.carry_events();
        self.start(Mode::Playing, from, false);
    }

    /// The playhead stopped by itself on the last frame: streams end, event sounds play out.
    pub fn finish(&mut self) {
        if self.mode == Mode::Playing && self.stopping.is_none() {
            self.carry_events();
            self.mode = Mode::Idle;
        }
    }

    /// Plays `frames` frames of the stream sounds at `from` (scrubbing). Event sounds playing
    /// out after the end carry on.
    pub fn scrub(&mut self, from: u64, frames: u64) {
        if self.stopping.is_some() {
            self.tails.clear();
        }
        self.start(Mode::Snippet { until: from.saturating_add(frames) }, from, true);
    }

    fn start(&mut self, mode: Mode, from: u64, streams_only: bool) {
        self.mode = mode;
        self.pos = from;
        self.started = from;
        self.stopping = None;
        self.opts = MixOptions { origin: from, streams_only };
    }

    /// Moves the event voices sounding at the playhead onto the tail clock.
    fn carry_events(&mut self) {
        if self.mode != Mode::Playing || self.opts.streams_only {
            return;
        }
        let (pos, origin, now) = (self.pos, self.opts.origin, self.tail_pos);
        let carried = self.voices.iter().filter(|v| v.kind == VoiceKind::Event && v.gain != 0.0 && v.at >= origin && v.at < pos && v.end > pos).map(|v| {
            let mut t = v.clone();
            t.at = now;
            t.end = now.saturating_add(v.end - pos);
            t.skip = v.skip.saturating_add(pos - v.at);
            t
        });
        self.tails.extend(carried);
        let over = self.tails.len().saturating_sub(MAX_TAILS);
        self.tails.drain(..over);
    }

    /// Fades out and goes quiet, event sounds playing out included.
    pub fn stop(&mut self) {
        if self.is_sounding() && self.stopping.is_none() {
            self.stopping = Some(RAMP_FRAMES);
        }
    }

    /// Playing (not scrubbing, not stopped).
    pub fn is_playing(&self) -> bool {
        self.mode == Mode::Playing && self.stopping.is_none()
    }

    /// Anything to hear: playing, scrubbing, or event sounds playing out.
    pub fn is_sounding(&self) -> bool {
        self.mode != Mode::Idle || !self.tails.is_empty()
    }

    /// The next output frame to be played.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// How far behind the device plays what it is handed, in output frames (from the device's
    /// timestamps; `0` when it doesn't say).
    pub fn set_latency(&mut self, frames: u64) {
        self.latency = frames.min(u64::from(self.rate));
    }

    /// The output frame being heard now, `since_fill` output frames of wall time after the
    /// last buffer was filled: the start of that buffer, less the device's latency, plus the
    /// time since (never past the buffer). This is the clock the playhead follows.
    pub fn heard(&self, since_fill: u64) -> u64 {
        let begun = self.pos.saturating_sub(self.last_fill).max(self.started);
        begun.saturating_add(since_fill.min(self.last_fill)).saturating_sub(self.latency).max(self.started)
    }

    /// Fills a device buffer of interleaved `channels`-channel frames: left and right on the
    /// first two channels (their average on a mono device), silence on any others, clipped to
    /// `-1..=1`.
    pub fn fill(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        self.last_fill = frames as u64;
        if !self.is_sounding() || frames == 0 {
            out.fill(0.0);
            self.gain_now = self.volume;
            return;
        }
        self.scratch.clear();
        self.scratch.resize(frames * 2, 0.0);
        if self.mode != Mode::Idle {
            mix(&self.voices, self.rate, self.opts, self.pos, &mut self.scratch);
        }
        let tails = !self.tails.is_empty();
        if tails {
            self.tail_scratch.resize(frames * 2, 0.0);
            mix(&self.tails, self.rate, MixOptions::default(), self.tail_pos, &mut self.tail_scratch);
        }
        let ramp = RAMP_FRAMES as f32;
        let glide = (self.volume - self.gain_now) / ramp;
        let mut done = false;
        for (k, (dst, src)) in out.chunks_exact_mut(channels).zip(self.scratch.as_chunks::<2>().0).enumerate() {
            let p = self.pos.saturating_add(k as u64);
            // The playhead's own sound: faded in at its start, out at a snippet's end.
            let mut env = match self.mode {
                Mode::Idle => 0.0,
                Mode::Playing => ((p - self.started) as f32 / ramp).min(1.0),
                Mode::Snippet { until } => ((p - self.started) as f32 / ramp).min(until.saturating_sub(p) as f32 / ramp).min(1.0),
            };
            let mut all = 1.0;
            if let Some(left) = &mut self.stopping {
                all = *left as f32 / ramp;
                *left = left.saturating_sub(1);
            }
            env = env.max(0.0);
            if (self.volume - self.gain_now).abs() > glide.abs() {
                self.gain_now += glide;
            } else {
                self.gain_now = self.volume;
            }
            let [mut l, mut r] = *src;
            l *= env;
            r *= env;
            if tails && let Some([tl, tr]) = self.tail_scratch.as_chunks::<2>().0.get(k) {
                l += tl;
                r += tr;
            }
            if done || all <= 0.0 {
                done = true;
                dst.fill(0.0);
                continue;
            }
            let g = all * self.gain_now;
            let (l, r) = ((l * g).clamp(-1.0, 1.0), (r * g).clamp(-1.0, 1.0));
            let (l, r) = (if l.is_finite() { l } else { 0.0 }, if r.is_finite() { r } else { 0.0 });
            match dst {
                [m] => *m = 0.5 * (l + r),
                [a, b, rest @ ..] => {
                    *a = l;
                    *b = r;
                    rest.fill(0.0);
                }
                [] => {}
            }
        }
        let n = frames as u64;
        self.pos = self.pos.saturating_add(n);
        self.tail_pos = self.tail_pos.saturating_add(n);
        let now = self.tail_pos;
        self.tails.retain(|t| t.end > now);
        if matches!(self.mode, Mode::Snippet { until } if self.pos >= until) {
            self.mode = Mode::Idle;
        }
        if done || self.stopping == Some(0) {
            self.mode = Mode::Idle;
            self.stopping = None;
            self.tails.clear();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{EnvPoint, Envelope, Pcm, VoiceKind};

    fn tone() -> Arc<Vec<Voice>> {
        let pcm = Arc::new(Pcm::new(8000, 1, vec![0.5; 8000]).unwrap());
        Arc::new(vec![Voice::new(pcm, VoiceKind::Stream, 0, 8000)])
    }

    #[test]
    fn plays_the_mix_after_a_short_ramp_and_advances() {
        let mut p = Player::new(8000);
        p.set_voices(tone());
        let mut buf = vec![1.0; 2 * 1024];
        p.fill(&mut buf, 2);
        assert!(buf.iter().all(|&s| s == 0.0), "idle is silent");
        p.play(100);
        p.fill(&mut buf, 2);
        assert_eq!(buf[0], 0.0, "starts from silence");
        assert!(buf[2] > 0.0 && buf[2] < 0.01);
        assert_eq!(&buf[2 * 300..2 * 302], &[0.5; 4], "then the mix itself");
        assert_eq!(p.position(), 1124);
        assert!(p.is_playing());
    }

    #[test]
    fn stops_with_a_fade_and_mono_and_surround_devices_get_the_right_layout() {
        let mut p = Player::new(8000);
        p.set_voices(tone());
        p.play(0);
        let mut buf = vec![0.0; 1024];
        p.fill(&mut buf, 1);
        assert_eq!(buf[1000], 0.5, "mono device: the average");
        let mut six = vec![9.0; 6 * 512];
        p.fill(&mut six, 6);
        assert_eq!(&six[6 * 10..6 * 11], &[0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
        p.stop();
        assert!(!p.is_playing());
        let mut buf = vec![0.0; 2 * 512];
        p.fill(&mut buf, 2);
        assert!(buf[0] > 0.49 && buf[2 * 200] < 0.2 && buf[2 * 300] == 0.0, "fades out over the ramp");
        p.fill(&mut buf, 2);
        assert!(buf.iter().all(|&s| s == 0.0), "then silent");
        assert!(!p.is_sounding());
    }

    #[test]
    fn a_scrub_snippet_plays_streams_briefly_then_stops() {
        let mut p = Player::new(8000);
        p.set_voices(tone());
        p.scrub(4000, 600);
        let mut buf = vec![0.0; 2 * 1000];
        p.fill(&mut buf, 2);
        assert_eq!(buf[2 * 300], 0.5);
        assert_eq!(buf[2 * 700], 0.0, "past the snippet");
        assert!(!p.is_playing());
        p.fill(&mut buf, 2);
        assert!(buf.iter().all(|&s| s == 0.0));
    }

    /// An event sound of 0.25 at frame 100, 2000 frames long, and a stream of 0.5 over 0..1000.
    fn event_and_stream() -> Arc<Vec<Voice>> {
        let quiet = Arc::new(Pcm::new(8000, 1, vec![0.25; 2000]).unwrap());
        let loud = Arc::new(Pcm::new(8000, 1, vec![0.5; 1000]).unwrap());
        Arc::new(vec![Voice::new(quiet, VoiceKind::Event, 100, 2100), Voice::new(loud, VoiceKind::Stream, 0, 1000)])
    }

    #[test]
    fn event_sounds_play_to_their_end_across_a_loop_and_after_the_last_frame() {
        let mut p = Player::new(8000);
        p.set_voices(event_and_stream());
        p.play(0);
        let mut buf = vec![0.0; 2 * 1000];
        p.fill(&mut buf, 2);
        assert_eq!(buf[2 * 500], 0.75, "event and stream together");
        // Looped playback goes round: the event (900 frames in) carries on beside the stream.
        p.wrap(0);
        p.fill(&mut buf, 2);
        assert_eq!(buf[2 * 50], 0.25 + 0.5 * 50.0 / 256.0, "the old event, the new stream ramping in");
        assert_eq!(buf[2 * 300], 0.25 + 0.5 + 0.25, "old event, stream and the event started again");
        // The playhead stops on the last frame: only events play on, to their ends.
        p.finish();
        assert!(!p.is_playing() && p.is_sounding());
        p.fill(&mut buf, 2);
        assert_eq!(buf[2 * 10], 0.5, "both events, no stream");
        assert_eq!(buf[2 * 200], 0.25, "the first event ended 1100 frames after the loop");
        for _ in 0..3 {
            p.fill(&mut buf, 2);
        }
        assert!(!p.is_sounding(), "everything played out");
        assert!(buf.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn stop_silences_event_sounds_playing_out_and_a_new_play_cuts_them() {
        let mut p = Player::new(8000);
        p.set_voices(event_and_stream());
        p.play(0);
        let mut buf = vec![0.0; 2 * 1000];
        p.fill(&mut buf, 2);
        p.finish();
        p.stop();
        p.fill(&mut buf, 2);
        assert!(buf[2 * 300..].iter().all(|&s| s == 0.0) && !p.is_sounding(), "Stop silences tails");
        p.play(0);
        p.fill(&mut buf, 2);
        p.wrap(0);
        p.play(0);
        p.fill(&mut buf, 2);
        assert_eq!(buf[2 * 50], 0.5 * 50.0 / 256.0, "a new start has no tails: only the stream, ramping in");
    }

    #[test]
    fn the_monitor_volume_glides_and_scales_everything() {
        let mut p = Player::new(8000);
        p.set_voices(tone());
        p.play(0);
        let mut buf = vec![0.0; 2 * 1000];
        p.fill(&mut buf, 2);
        p.set_volume(0.5);
        p.fill(&mut buf, 2);
        assert!(buf[0] > 0.49, "glides, no jump");
        assert!(buf[2 * 100] < 0.49 && buf[2 * 100] > 0.25);
        assert_eq!(buf[2 * 400], 0.25);
        p.set_volume(0.0);
        p.fill(&mut buf, 2);
        assert_eq!(buf[2 * 400], 0.0, "mute");
        assert!(p.is_playing(), "muted still keeps the clock");
        p.set_volume(f32::NAN);
        assert_eq!(p.volume(), 0.0);
        p.set_volume(7.0);
        assert_eq!(p.volume(), 1.0);
    }

    #[test]
    fn the_heard_frame_trails_the_mix_by_the_buffer_and_the_latency() {
        let mut p = Player::new(8000);
        p.set_voices(tone());
        p.play(1000);
        assert_eq!(p.heard(0), 1000, "nothing played yet: the start");
        let mut buf = vec![0.0; 2 * 400];
        p.fill(&mut buf, 2);
        p.fill(&mut buf, 2);
        assert_eq!(p.position(), 1800);
        assert_eq!(p.heard(0), 1400, "the last buffer's start");
        assert_eq!(p.heard(100), 1500);
        assert_eq!(p.heard(10_000), 1800, "never past what was handed over");
        p.set_latency(160);
        assert_eq!(p.heard(0), 1240);
        p.set_latency(u64::MAX);
        assert_eq!(p.heard(0), 1000, "never before the start");
    }

    /// Hostile voices: envelopes at the ends of time, non-finite gains, huge skips, inverted
    /// spans, every buffer and channel shape. Nothing panics and every sample is finite in
    /// `-1..=1`.
    #[test]
    fn hostile_envelopes_and_voices_never_panic_or_leave_the_range() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let odd = [0, 1, 2, 255, 256, 1 << 20, u64::MAX / 3, u64::MAX - 1, u64::MAX];
        let gains = [0.0, 1.0, -1.0, 0.5, 1e30, -1e30, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MIN_POSITIVE];
        for round in 0..300 {
            let rate = [8000, 1, 44_100, 48_000, 192_000][round % 5];
            let pcm = Arc::new(Pcm::new([8000, 22_050, 48_000][round % 3], 1 + (round % 2) as u16, (0..64).map(|i| ((i as f32) * 0.37).sin() * 4.0).collect()).unwrap());
            let mut voices = Vec::new();
            for _ in 0..1 + next() % 4 {
                let pick = |n: u64| odd[(n % odd.len() as u64) as usize];
                let mut v = Voice::new(pcm.clone(), if next() % 2 == 0 { VoiceKind::Event } else { VoiceKind::Stream }, pick(next()) % 5000, pick(next()));
                v.skip = pick(next());
                v.gain = gains[(next() % gains.len() as u64) as usize];
                let points = (0..next() % 20).map(|_| EnvPoint { at: pick(next()), left: gains[(next() % gains.len() as u64) as usize], right: gains[(next() % gains.len() as u64) as usize] }).collect();
                v.envelope = Envelope::new(points).map(Arc::new);
                voices.push(v);
            }
            let mut p = Player::new(rate);
            p.set_voices(Arc::new(voices));
            p.set_volume(gains[round % gains.len()]);
            p.set_latency(next());
            match round % 4 {
                0 => p.play(next() % 10_000),
                1 => p.scrub(odd[round % odd.len()], next() % 4000),
                _ => p.play(odd[round % odd.len()]),
            }
            for i in 0..4 {
                let ch = 1 + (next() % 6) as usize;
                let mut buf = vec![0.0; ch * (next() % 700) as usize + (i % 3)];
                p.fill(&mut buf, ch);
                assert!(buf.iter().all(|s| s.is_finite() && (-1.0..=1.0).contains(s)), "round {round}");
                if i == 1 {
                    p.wrap(next() % 3000);
                }
                if i == 2 {
                    p.finish();
                }
                let _ = p.heard(next());
            }
        }
    }
}
