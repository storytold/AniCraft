//! Sound during stage playback (ADR 0020). The movie's sounds are laid out by the engine
//! (`doc::sound::voices`, pure), handed to an `anicraft_audio::Player` that the audio device's
//! callback pulls from, and the playhead follows the player's clock while sound plays, so
//! picture keeps time with sound.
//!
//! The device (cpal: WASAPI, CoreAudio, ALSA) opens on first need: never at start-up, never in
//! `--screenshot` runs or tests. If it can't open, or fails later, playback carries on silently
//! on the wall clock.

use anicraft_audio::{Pcm, Player, Voice, VoiceKind};
use anicraft_engine::StateId;
use anicraft_engine::doc::{Document, sound};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct SoundOut {
    player: Arc<Mutex<Player>>,
    failed: Arc<AtomicBool>,
    #[cfg(sound_output)]
    stream: Option<cpal::Stream>,
    tried: bool,
    rate: u32,
    /// The document state and frame rate the player's voices were laid out for.
    laid_out: Option<(StateId, u32)>,
    /// A library preview is loaded instead of the movie's sounds.
    previewing: bool,
    /// Why there is no sound, once.
    pub error: Option<String>,
}

impl SoundOut {
    pub fn new() -> SoundOut {
        SoundOut {
            player: Arc::new(Mutex::new(Player::new(48_000))),
            failed: Arc::new(AtomicBool::new(false)),
            #[cfg(sound_output)]
            stream: None,
            tried: false,
            rate: 48_000,
            laid_out: None,
            previewing: false,
            error: None,
        }
    }

    /// Opens the output device (the first time only). Whether sound can be heard.
    fn ready(&mut self) -> bool {
        if !self.tried {
            self.tried = true;
            match open(&self.player, &self.failed) {
                #[cfg(sound_output)]
                Ok((stream, rate)) => {
                    self.stream = Some(stream);
                    self.rate = rate;
                    if let Ok(mut p) = self.player.lock() {
                        *p = Player::new(rate);
                    }
                }
                #[cfg(not(sound_output))]
                Ok(rate) => self.rate = rate,
                Err(e) => self.error = Some(e),
            }
        }
        self.live()
    }

    /// The device is open and working.
    fn live(&self) -> bool {
        #[cfg(sound_output)]
        let open = self.stream.is_some();
        #[cfg(not(sound_output))]
        let open = false;
        open && !self.failed.load(Ordering::Relaxed)
    }

    fn with_player<R>(&self, f: impl FnOnce(&mut Player) -> R) -> Option<R> {
        self.player.lock().ok().map(|mut p| f(&mut p))
    }

    /// Lays the document's sounds out again if it changed since (edits are heard at once,
    /// even mid-playback).
    pub fn sync(&mut self, doc: &Document, state: StateId) {
        let key = (state, doc.stage.fps.to_bits());
        if self.laid_out == Some(key) && !self.previewing {
            return;
        }
        let voices = Arc::new(sound::voices(doc, self.rate));
        self.previewing = false;
        self.laid_out = Some(key);
        self.with_player(|p| p.set_voices(voices));
    }

    /// Starts sound from movie frame `frame`; whether it plays (the device is there and the
    /// movie has sounds), in which case the playhead should follow [`SoundOut::frame`].
    pub fn play(&mut self, doc: &Document, state: StateId, frame: u32) -> bool {
        if !sound::has_sounds(doc) || !self.ready() {
            return false;
        }
        self.sync(doc, state);
        let at = sound::frame_start(u64::from(frame), doc.stage.fps, self.rate);
        self.with_player(|p| p.play(at)).is_some()
    }

    /// A one-frame snippet of the stream sounds at movie frame `frame` (scrubbing).
    pub fn scrub(&mut self, doc: &Document, state: StateId, frame: u32) {
        if !sound::has_streams(doc) || !self.ready() {
            return;
        }
        self.sync(doc, state);
        let fps = doc.stage.fps;
        let at = sound::frame_start(u64::from(frame), fps, self.rate);
        let len = sound::frame_start(u64::from(frame) + 1, fps, self.rate).saturating_sub(at);
        self.with_player(|p| p.scrub(at, len));
    }

    /// Plays a library sound on its own (the Library's preview button).
    pub fn preview(&mut self, pcm: Arc<Pcm>) {
        let end = Voice::play_frames(&pcm, self.rate);
        self.preview_voice(Voice::new(pcm, VoiceKind::Event, 0, end));
    }

    /// Plays one voice on its own from its start (a library sound, or a keyframe's sound with
    /// its effect from the envelope editor).
    pub fn preview_voice(&mut self, voice: Voice) {
        if !self.ready() {
            return;
        }
        self.previewing = true;
        self.with_player(|p| {
            p.set_voices(Arc::new(vec![voice]));
            p.play(0);
        });
    }

    /// The output rate (what voices for [`SoundOut::preview_voice`] and
    /// [`SoundOut::set_movie`] are laid out at).
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Test Movie's sound (ADR 0039): plays `voices` (laid out by the player as the movie
    /// runs) from output frame `from`. Whether sound can be heard.
    pub fn play_movie(&mut self, voices: Arc<Vec<Voice>>, from: u64) -> bool {
        if !self.ready() {
            return false;
        }
        self.previewing = true;
        self.with_player(|p| {
            p.set_voices(voices);
            p.play(from);
        })
        .is_some()
    }

    /// New voices for Test Movie, carrying on from where it plays.
    pub fn set_movie(&mut self, voices: Arc<Vec<Voice>>) {
        if self.previewing {
            self.with_player(|p| p.set_voices(voices));
        }
    }

    /// The output frame Test Movie's sound has reached while it plays.
    pub fn movie_position(&self) -> Option<u64> {
        if !self.previewing || !self.is_playing() {
            return None;
        }
        self.with_player(|p| p.position())
    }

    /// Fades out and goes quiet.
    pub fn stop(&mut self) {
        self.with_player(Player::stop);
    }

    pub fn is_playing(&self) -> bool {
        self.live() && self.with_player(|p| p.is_playing()).unwrap_or(false)
    }

    /// The movie frame the sound has reached while it plays (`None` when it doesn't: the
    /// playhead then runs on the wall clock).
    pub fn frame(&self, fps: f32) -> Option<u32> {
        if self.previewing || !self.is_playing() {
            return None;
        }
        let pos = self.with_player(|p| p.position())?;
        let fps = if fps.is_finite() && fps > 0.0 { f64::from(fps) } else { 24.0 };
        Some((pos as f64 * fps / f64::from(self.rate.max(1))).floor().min(f64::from(u32::MAX)) as u32)
    }
}

#[cfg(sound_output)]
fn open(player: &Arc<Mutex<Player>>, failed: &Arc<AtomicBool>) -> Result<(cpal::Stream, u32), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let device = cpal::default_host().default_output_device().ok_or("no sound output device")?;
    let supported = device.default_output_config().map_err(|e| format!("the sound device has no usable format ({e})"))?;
    let config = supported.config();
    let rate = config.sample_rate;
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, player, failed),
        cpal::SampleFormat::I16 => build::<i16>(&device, config, player, failed),
        cpal::SampleFormat::U16 => build::<u16>(&device, config, player, failed),
        cpal::SampleFormat::I32 => build::<i32>(&device, config, player, failed),
        other => return Err(format!("the sound device wants {other:?} samples, which AniCraft doesn't write yet")),
    }
    .map_err(|e| format!("could not start sound ({e})"))?;
    stream.play().map_err(|e| format!("could not start sound ({e})"))?;
    Ok((stream, rate))
}

#[cfg(not(sound_output))]
fn open(_: &Arc<Mutex<Player>>, _: &Arc<AtomicBool>) -> Result<u32, String> {
    Err(if cfg!(target_arch = "wasm32") { "the web version of AniCraft has no sound output yet".into() } else { "this build of AniCraft has no sound output".into() })
}

/// An output stream whose callback mixes from the player. The lock is held only while one
/// buffer is mixed; the UI holds it only to change a few fields.
#[cfg(sound_output)]
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(device: &cpal::Device, config: cpal::StreamConfig, player: &Arc<Mutex<Player>>, failed: &Arc<AtomicBool>) -> Result<cpal::Stream, cpal::Error> {
    use cpal::traits::DeviceTrait;
    let channels = usize::from(config.channels);
    let (player, failed) = (player.clone(), failed.clone());
    let mut buf: Vec<f32> = Vec::new();
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            buf.resize(data.len(), 0.0);
            match player.lock() {
                Ok(mut p) => p.fill(&mut buf, channels),
                Err(_) => buf.fill(0.0),
            }
            for (d, s) in data.iter_mut().zip(&buf) {
                *d = T::from_sample(*s);
            }
        },
        // A rerouted stream or a refused real-time boost still plays; anything else ends it.
        move |e: cpal::Error| {
            if !matches!(e.kind(), cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::DeviceBusy) {
                failed.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}
