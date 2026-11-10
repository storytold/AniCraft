//! The device-independent half of playback. The shell's audio callback calls
//! [`Player::fill`]; the UI tells the player where to play from, and reads back how far it has
//! got (the audio clock drives the stage during playback, so picture follows sound).
//!
//! What it plays is [`mix`](crate::mix) of the current voices, so playback and export agree
//! sample for sample, apart from short ramps at starts and stops that keep them click-free.

use crate::mix::{MixOptions, Voice, mix};
use std::sync::Arc;

/// Length of the fade at a start or a stop (about 5 ms at 48 kHz).
pub const RAMP_FRAMES: u64 = 256;

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
    scratch: Vec<f32>,
}

impl Player {
    /// A silent player for output at `rate`.
    pub fn new(rate: u32) -> Player {
        Player { voices: Arc::new(Vec::new()), rate: rate.max(1), mode: Mode::Idle, opts: MixOptions::default(), pos: 0, started: 0, stopping: None, scratch: Vec::new() }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// What there is to hear (laid out for this player's rate). Playback carries on from the
    /// same position, so an edit during playback is heard at once.
    pub fn set_voices(&mut self, voices: Arc<Vec<Voice>>) {
        self.voices = voices;
    }

    /// Plays from output frame `from`: stream sounds from there, event sounds as their
    /// keyframes are reached.
    pub fn play(&mut self, from: u64) {
        self.start(Mode::Playing, from, false);
    }

    /// Plays `frames` frames of the stream sounds at `from` (scrubbing).
    pub fn scrub(&mut self, from: u64, frames: u64) {
        self.start(Mode::Snippet { until: from.saturating_add(frames) }, from, true);
    }

    fn start(&mut self, mode: Mode, from: u64, streams_only: bool) {
        self.mode = mode;
        self.pos = from;
        self.started = from;
        self.stopping = None;
        self.opts = MixOptions { origin: from, streams_only };
    }

    /// Fades out and goes quiet.
    pub fn stop(&mut self) {
        if self.mode != Mode::Idle && self.stopping.is_none() {
            self.stopping = Some(RAMP_FRAMES);
        }
    }

    /// Playing (not scrubbing, not stopped).
    pub fn is_playing(&self) -> bool {
        self.mode == Mode::Playing && self.stopping.is_none()
    }

    /// The next output frame to be played.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Fills a device buffer of interleaved `channels`-channel frames: left and right on the
    /// first two channels (their average on a mono device), silence on any others, clipped to
    /// `-1..=1`.
    pub fn fill(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let frames = out.len() / channels;
        if self.mode == Mode::Idle || frames == 0 {
            out.fill(0.0);
            return;
        }
        self.scratch.resize(frames * 2, 0.0);
        mix(&self.voices, self.rate, self.opts, self.pos, &mut self.scratch);
        let ramp = RAMP_FRAMES as f32;
        let mut done = false;
        for (k, (dst, src)) in out.chunks_exact_mut(channels).zip(self.scratch.as_chunks::<2>().0).enumerate() {
            let p = self.pos + k as u64;
            let mut env = ((p - self.started) as f32 / ramp).min(1.0);
            if let Mode::Snippet { until } = self.mode {
                env = env.min(until.saturating_sub(p) as f32 / ramp);
            }
            if let Some(left) = &mut self.stopping {
                env = env.min(*left as f32 / ramp);
                *left = left.saturating_sub(1);
            }
            if done || env <= 0.0 {
                done |= self.stopping == Some(0) || matches!(self.mode, Mode::Snippet { until } if p >= until);
                dst.fill(0.0);
                continue;
            }
            let [l, r] = *src;
            let (l, r) = ((l * env).clamp(-1.0, 1.0), (r * env).clamp(-1.0, 1.0));
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
        self.pos = self.pos.saturating_add(frames as u64);
        if done || self.stopping == Some(0) || matches!(self.mode, Mode::Snippet { until } if self.pos >= until) {
            self.mode = Mode::Idle;
            self.stopping = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{Pcm, VoiceKind};

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
}
