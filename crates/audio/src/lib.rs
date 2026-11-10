//! AniCraft audio (layer L0): sounds as data, and the pure functions that turn them into
//! samples.
//!
//! * [`decode`] reads WAV (own reader), Ogg Vorbis (`lewton`) and MP3 (`nanomp3`) into [`Pcm`]:
//!   32-bit float frames, mono or stereo, bounded in length.
//! * [`mix`] sums [`Voice`]s (a sound placed in time) into stereo output at any rate,
//!   sample-exactly when the rates match and with cubic Hermite interpolation when they don't.
//!   Mixing is a pure function of its inputs: export, playback and tests hear the same thing.
//! * [`wav`] writes WAV files; [`Peaks`] summarises a sound for waveform drawing; [`Player`]
//!   is the device-independent half of playback (an audio callback calls [`Player::fill`]).
//!
//! Nothing here opens an audio device: the desktop shell owns that (ADR 0020).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable, clippy::indexing_slicing)]

pub mod mix;
mod mp3;
mod peaks;
mod player;
mod vorbis;
pub mod wav;

pub use mix::{EnvPoint, Envelope, MixOptions, Voice, VoiceKind, mix};
pub use peaks::Peaks;
pub use player::{Player, RAMP_FRAMES};

/// Most sample values (all channels) a decoded sound may hold: 256 MB of `f32`, about 12
/// minutes of 44.1 kHz stereo or 25 minutes of mono.
pub const MAX_SAMPLES: usize = 64 * 1024 * 1024;
/// Sample rates accepted from files.
pub const MIN_RATE: u32 = 1_000;
pub const MAX_RATE: u32 = 384_000;

/// Decoded audio: interleaved `f32` samples in `-1..=1` (louder material is kept, not
/// clipped), one or two channels.
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    /// 1 (mono) or 2 (stereo: left, right).
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Pcm {
    /// Validates the shape: 1 or 2 channels, a rate in range, whole frames, at most
    /// [`MAX_SAMPLES`]. Non-finite samples become silence.
    pub fn new(rate: u32, channels: u16, mut samples: Vec<f32>) -> Result<Pcm, AudioError> {
        if !(1..=2).contains(&channels) {
            return Err(AudioError::Unsupported(format!("{channels} channels")));
        }
        if !(MIN_RATE..=MAX_RATE).contains(&rate) {
            return Err(AudioError::Unsupported(format!("a sample rate of {rate} Hz")));
        }
        if samples.len() > MAX_SAMPLES {
            return Err(AudioError::TooLong);
        }
        samples.truncate(samples.len() - samples.len() % usize::from(channels));
        if samples.is_empty() {
            return Err(AudioError::Empty);
        }
        for s in &mut samples {
            if !s.is_finite() {
                *s = 0.0;
            }
        }
        Ok(Pcm { rate, channels, samples })
    }

    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// Length in seconds.
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.rate.max(1))
    }

    /// Frame `i` as (left, right); mono plays on both sides. Silence outside the sound.
    #[inline]
    pub fn frame(&self, i: usize) -> (f32, f32) {
        if self.channels == 2 {
            match self.samples.get(2 * i..2 * i + 2) {
                Some([l, r]) => (*l, *r),
                _ => (0.0, 0.0),
            }
        } else {
            let m = self.samples.get(i).copied().unwrap_or(0.0);
            (m, m)
        }
    }
}

/// The encodings sounds arrive in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Wav,
    Vorbis,
    Mp3,
}

impl Format {
    /// The name used in files and commands.
    pub fn key(self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Vorbis => "ogg",
            Format::Mp3 => "mp3",
        }
    }

    pub fn from_key(s: &str) -> Option<Format> {
        match s.trim().to_ascii_lowercase().as_str() {
            "wav" | "wave" => Some(Format::Wav),
            "ogg" | "oga" | "vorbis" => Some(Format::Vorbis),
            "mp3" => Some(Format::Mp3),
            _ => None,
        }
    }

    /// Told apart by content, not by file name.
    pub fn sniff(data: &[u8]) -> Option<Format> {
        if data.len() >= 12 && data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WAVE") {
            return Some(Format::Wav);
        }
        if data.starts_with(b"OggS") {
            return Some(Format::Vorbis);
        }
        if data.starts_with(b"ID3") || nanomp3::detect(data) {
            return Some(Format::Mp3);
        }
        None
    }
}

/// Why sound data can't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// Not WAV, Ogg Vorbis or MP3.
    UnknownFormat,
    /// A file of a known kind that is damaged.
    Unreadable,
    /// A valid file using something not supported (e.g. a compressed WAV codec).
    Unsupported(String),
    /// Longer than [`MAX_SAMPLES`].
    TooLong,
    /// No audio in it.
    Empty,
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::UnknownFormat => f.write_str("not a WAV, Ogg Vorbis or MP3 sound"),
            AudioError::Unreadable => f.write_str("a damaged or unsupported sound file"),
            AudioError::Unsupported(what) => write!(f, "sounds with {what} are not supported yet"),
            AudioError::TooLong => write!(f, "the sound is too long (the limit is {} samples, about 12 minutes of stereo)", MAX_SAMPLES),
            AudioError::Empty => f.write_str("the file holds no audio"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Decodes a WAV, Ogg Vorbis or MP3 file (told apart by content).
pub fn decode(data: &[u8]) -> Result<Pcm, AudioError> {
    match Format::sniff(data).ok_or(AudioError::UnknownFormat)? {
        Format::Wav => wav::decode(data),
        Format::Vorbis => guarded(|| vorbis::decode(data)),
        Format::Mp3 => guarded(|| mp3::decode(data)),
    }
}

/// Runs a third-party decoder on untrusted data: should it panic on a malformed file, the
/// file is reported unreadable instead of taking the app down (golden rule 1).
fn guarded(f: impl FnOnce() -> Result<Pcm, AudioError>) -> Result<Pcm, AudioError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(Err(AudioError::Unreadable))
}

/// Collects decoded interleaved samples with `channels` channels, keeping at most two (the
/// first two: front left and right in WAV and Vorbis order), within [`MAX_SAMPLES`].
pub(crate) struct Collector {
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl Collector {
    pub fn new(channels: usize) -> Collector {
        Collector { channels: channels.max(1), samples: Vec::new() }
    }

    pub fn out_channels(&self) -> u16 {
        if self.channels >= 2 { 2 } else { 1 }
    }

    /// Appends whole frames from `src`; `Err(TooLong)` past the limit.
    pub fn push(&mut self, src: &[f32]) -> Result<(), AudioError> {
        let keep = usize::from(self.out_channels());
        if self.samples.len() + src.len() / self.channels * keep > MAX_SAMPLES {
            return Err(AudioError::TooLong);
        }
        if self.channels == keep {
            self.samples.extend_from_slice(src);
        } else {
            for frame in src.chunks_exact(self.channels) {
                self.samples.extend(frame.iter().take(keep));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_told_apart_by_content() {
        let wav = wav::encode(8000, 1, &[0.0, 0.5], wav::Bits::Int16);
        assert_eq!(Format::sniff(&wav), Some(Format::Wav));
        assert_eq!(Format::sniff(b"OggS\0\x02"), Some(Format::Vorbis));
        assert_eq!(Format::sniff(b"ID3\x04\0\0\0\0\0\0"), Some(Format::Mp3));
        assert_eq!(Format::sniff(b"hello"), None);
        assert_eq!(decode(b"hello"), Err(AudioError::UnknownFormat));
        assert_eq!(Format::from_key("WAV"), Some(Format::Wav));
        assert_eq!(Format::from_key(Format::Vorbis.key()), Some(Format::Vorbis));
    }

    #[test]
    fn damaged_files_of_every_kind_are_errors_not_crashes() {
        let wav = wav::encode(8000, 2, &[0.25; 64], wav::Bits::Int16);
        for cut in [0, 4, 12, 20, 36, 44, 45, 50] {
            let _ = decode(&wav[..cut.min(wav.len())]);
        }
        assert!(decode(b"OggS\0\x02\0\0\0\0\0\0\0\0junkjunkjunk").is_err());
        assert!(decode(b"ID3\x04\0\0\0\0\0\x10\xff\xfb\x90\x00junk").is_err());
        // Random bytes behind each signature.
        let mut x = 0x2545_f491u32;
        for sig in [&b"RIFF\0\0\0\0WAVEfmt "[..], b"OggS", b"ID3\x03\0\0\0\0\0\0"] {
            for _ in 0..50 {
                let mut d = sig.to_vec();
                for _ in 0..400 {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    d.push(x as u8);
                }
                let _ = decode(&d);
            }
        }
    }

    /// Half a second of a 440 Hz sine at 1/16 scale, 22.05 kHz; the fixtures were made for
    /// these tests (see ATTRIBUTION.md).
    fn check_tone(p: &Pcm, channels: u16) {
        assert_eq!((p.rate, p.channels), (22_050, channels));
        let secs = p.seconds();
        assert!((0.49..0.56).contains(&secs), "{secs} s");
        let peak = p.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((0.04..0.08).contains(&peak), "peak {peak}");
        // 440 Hz: about 2 × 440 × 0.5 zero crossings in the left channel.
        let left: Vec<f32> = p.samples.iter().step_by(usize::from(channels)).copied().collect();
        let crossings = left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((420..520).contains(&crossings), "{crossings} crossings");
    }

    #[test]
    fn ogg_vorbis_decodes() {
        let data = include_bytes!("../tests/data/tone.ogg");
        assert_eq!(Format::sniff(data), Some(Format::Vorbis));
        check_tone(&decode(data).unwrap(), 2);
        // Cut short (the audio is in one page here): an error, never a crash.
        for cut in [1, 3, 5, 9] {
            let _ = decode(&data[..data.len() * cut / 10]);
        }
    }

    #[test]
    fn mp3_decodes() {
        let data = include_bytes!("../tests/data/tone.mp3");
        assert_eq!(Format::sniff(data), Some(Format::Mp3));
        let p = decode(data).unwrap();
        check_tone(&p, 1);
        // This file's tag doesn't record the encoder delay, so the decoder's 529 samples stay.
        let first = p.samples.iter().position(|s| s.abs() > 0.01).unwrap();
        assert!(first < 600, "first sound at sample {first}");
        let cut = decode(&data[..data.len() / 2]).unwrap();
        assert!(cut.seconds() < 0.4, "a cut file keeps what decoded");
    }

    #[test]
    fn pcm_validates_its_shape() {
        assert!(Pcm::new(44_100, 3, vec![0.0; 3]).is_err());
        assert!(Pcm::new(10, 1, vec![0.0; 3]).is_err());
        assert_eq!(Pcm::new(8000, 2, vec![]), Err(AudioError::Empty));
        let p = Pcm::new(8000, 2, vec![0.1, 0.2, f32::NAN, 0.4, 0.9]).unwrap();
        assert_eq!(p.samples, vec![0.1, 0.2, 0.0, 0.4], "a partial frame is dropped, NaN is silence");
        assert_eq!(p.frames(), 2);
        assert_eq!(p.frame(1), (0.0, 0.4));
        assert_eq!(p.frame(9), (0.0, 0.0));
        let m = Pcm::new(8000, 1, vec![0.3]).unwrap();
        assert_eq!(m.frame(0), (0.3, 0.3));
    }

    #[test]
    fn collector_keeps_the_front_pair_of_surround_sound() {
        let mut c = Collector::new(6);
        c.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0]).unwrap();
        assert_eq!(c.samples, vec![1.0, 2.0, 7.0, 8.0]);
        assert_eq!(c.out_channels(), 2);
    }
}
