//! Sound during stage playback (ADR 0020, ADR 0095). The movie's sounds are laid out by the
//! engine (`doc::sound::voices`, pure), handed to an `anicraft_audio::Player` that the audio
//! device's callback pulls from, and the playhead follows the sound's clock while it plays: the
//! frame shown is the one being *heard* (the device's latency taken off), so picture keeps time
//! with sound and a slow frame drops frames instead of drifting.
//!
//! Where the sound goes is an [`Output`]: the system's device through cpal (WASAPI, CoreAudio,
//! ALSA, Web Audio in the browser), or a fake one in tests, so the clock and the mixing paths are
//! tested without a device. The device opens on first need: never at start-up, never in
//! `--screenshot` runs or tests. If there is none, or it goes away mid-play, playback carries on
//! silently on the wall clock with a notice and a log line, and AniCraft tries again a little
//! later (a device plugged in, or a new default device, is picked up without a restart).

use anicraft_audio::{Pcm, Player, Voice, VoiceKind};
use anicraft_engine::StateId;
use anicraft_engine::doc::{Document, sound};
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Seconds between tries at opening a device after one failed or went away.
const RETRY_SECS: f64 = 2.0;

/// The shortest scrub snippet, in seconds (one frame at high frame rates is too short to hear).
const MIN_SCRUB_SECS: f64 = 0.05;

/// What the device's callback and the UI share: the player, and when it last filled a buffer
/// (seconds on the output's clock), so the UI can tell how far into that buffer the device is.
pub struct Shared {
    pub player: Player,
    pub filled_at: f64,
}

/// A place sound can go.
pub trait Output {
    /// Opens a stream whose callback fills buffers from `shared` (setting `filled_at`, and the
    /// player's latency when the device says), and sets `failed` if the stream dies. Returns
    /// what keeps the stream alive and its sample rate, or why there is no sound.
    fn open(&mut self, shared: &Arc<Mutex<Shared>>, failed: &Arc<AtomicBool>) -> Result<(Box<dyn Any>, u32), String>;
    /// Seconds on a monotonic clock (the one `filled_at` is on).
    fn now(&self) -> f64;
}

pub struct SoundOut {
    shared: Arc<Mutex<Shared>>,
    failed: Arc<AtomicBool>,
    output: Box<dyn Output>,
    /// Keeps the device's stream open.
    stream: Option<Box<dyn Any>>,
    /// When to try opening a device again (`None`: not yet tried).
    retry_at: Option<f64>,
    rate: u32,
    /// The monitor volume (`0..=1`, 0 when muted), kept across reopened devices.
    volume: f32,
    /// The document state and frame rate the player's voices were laid out for.
    laid_out: Option<(StateId, u32)>,
    /// A library preview or Test Movie is loaded instead of the movie's sounds.
    previewing: bool,
    /// The last problem said, so the same one isn't said again on every retry.
    said: Option<String>,
    /// Why there is no sound, to show once (taken by the caller).
    pub error: Option<String>,
}

impl Default for SoundOut {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundOut {
    /// Sound to the system's device.
    pub fn new() -> SoundOut {
        Self::with_output(Box::new(Device::new()))
    }

    /// Sound to `output` (tests pass a fake device).
    pub fn with_output(output: Box<dyn Output>) -> SoundOut {
        SoundOut {
            shared: Arc::new(Mutex::new(Shared { player: Player::new(48_000), filled_at: 0.0 })),
            failed: Arc::new(AtomicBool::new(false)),
            output,
            stream: None,
            retry_at: None,
            rate: 48_000,
            volume: 1.0,
            laid_out: None,
            previewing: false,
            said: None,
            error: None,
        }
    }

    fn say(&mut self, e: String) {
        if self.said.as_ref() != Some(&e) {
            // A log line for bug reports, and a notice for the artist (shown by the caller).
            eprintln!("anicraft: sound: {e}");
            self.said = Some(e.clone());
            self.error = Some(e);
        }
    }

    /// Notices a device that went away: closes its stream and says so (once), so the next
    /// play opens the system's device again.
    pub fn check(&mut self) {
        if self.lost() {
            self.stream = None;
            self.retry_at = Some(self.output.now());
            self.say(tr!("the sound device stopped working").to_string());
        }
    }

    /// Opens the output device if it isn't, or reopens it after it went away (no more often
    /// than every few seconds). Whether sound can be heard.
    fn ready(&mut self) -> bool {
        self.check();
        if self.stream.is_some() {
            return true;
        }
        let now = self.output.now();
        if self.retry_at.is_some_and(|t| now < t) {
            return false;
        }
        self.retry_at = Some(now + RETRY_SECS);
        self.failed.store(false, Ordering::Relaxed);
        match self.output.open(&self.shared, &self.failed) {
            Ok((stream, rate)) => {
                self.stream = Some(stream);
                let rate = rate.max(1);
                if let Ok(mut s) = self.shared.lock() {
                    if rate != self.rate || s.player.rate() != rate {
                        s.player = Player::new(rate);
                        self.laid_out = None;
                    }
                    s.player.set_volume(self.volume);
                }
                self.rate = rate;
                if self.said.take().is_some() {
                    eprintln!("anicraft: sound: output is back");
                }
                true
            }
            Err(e) => {
                self.say(e);
                false
            }
        }
    }

    /// The device is open and working.
    fn live(&self) -> bool {
        self.stream.is_some() && !self.failed.load(Ordering::Relaxed)
    }

    /// The device went away since sound last started (the caller carries on silently).
    pub fn lost(&self) -> bool {
        self.stream.is_some() && self.failed.load(Ordering::Relaxed)
    }

    fn with_player<R>(&self, f: impl FnOnce(&mut Player) -> R) -> Option<R> {
        self.shared.lock().ok().map(|mut s| f(&mut s.player))
    }

    /// The monitor volume, `0..=1` (0: muted). Playback, scrubbing, previews and Test Movie;
    /// never exports.
    pub fn set_volume(&mut self, volume: f32) {
        if volume.to_bits() != self.volume.to_bits() {
            self.volume = volume;
            self.with_player(|p| p.set_volume(volume));
        }
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

    /// Looped playback went round to movie frame `frame`: sound carries on from there, the
    /// event sounds still sounding playing to their end.
    pub fn wrap(&mut self, doc: &Document, state: StateId, frame: u32) {
        self.sync(doc, state);
        let at = sound::frame_start(u64::from(frame), doc.stage.fps, self.rate);
        self.with_player(|p| p.wrap(at));
    }

    /// The playhead stopped by itself on the last frame: event sounds play out.
    pub fn finish(&mut self) {
        self.with_player(Player::finish);
    }

    /// A short snippet of the stream sounds at movie frame `frame` (scrubbing): one frame, or
    /// a twentieth of a second at high frame rates.
    pub fn scrub(&mut self, doc: &Document, state: StateId, frame: u32) {
        if !sound::has_streams(doc) || !self.ready() {
            return;
        }
        self.sync(doc, state);
        let fps = doc.stage.fps;
        let at = sound::frame_start(u64::from(frame), fps, self.rate);
        let one = sound::frame_start(u64::from(frame) + 1, fps, self.rate).saturating_sub(at);
        let len = one.max((f64::from(self.rate) * MIN_SCRUB_SECS) as u64);
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

    /// The output frame Test Movie's sound is heard at while it plays.
    pub fn movie_position(&self) -> Option<u64> {
        if !self.previewing || !self.is_playing() {
            return None;
        }
        self.heard()
    }

    /// Fades out and goes quiet (event sounds playing out included).
    pub fn stop(&mut self) {
        self.with_player(Player::stop);
    }

    pub fn is_playing(&self) -> bool {
        self.live() && self.with_player(|p| p.is_playing()).unwrap_or(false)
    }

    /// Anything to hear: playing, a scrub snippet, or event sounds playing out.
    #[cfg(test)]
    pub fn is_sounding(&self) -> bool {
        self.live() && self.with_player(|p| p.is_sounding()).unwrap_or(false)
    }

    /// The output frame being heard now.
    fn heard(&self) -> Option<u64> {
        let now = self.output.now();
        let s = self.shared.lock().ok()?;
        let since = ((now - s.filled_at).max(0.0) * f64::from(self.rate)).min(1e12) as u64;
        Some(s.player.heard(since))
    }

    /// The movie frame being heard while the sound plays (`None` when it doesn't: the
    /// playhead then runs on the wall clock).
    pub fn frame(&self, fps: f32) -> Option<u32> {
        if self.previewing || !self.is_playing() {
            return None;
        }
        Some(frame_at(self.heard()?, fps, self.rate))
    }
}

/// The movie frame output frame `pos` is in: the last frame whose start
/// ([`sound::frame_start`], the layout's own rounding) is at or before it.
pub fn frame_at(pos: u64, fps: f32, rate: u32) -> u32 {
    let rate = rate.max(1);
    let f = if fps.is_finite() && fps > 0.0 { f64::from(fps) } else { 24.0 };
    let mut n = (pos as f64 * f / f64::from(rate)).floor().clamp(0.0, f64::from(u32::MAX)) as u64;
    // The estimate is within a frame of the answer; a few steps settle the rounding.
    for _ in 0..4 {
        if n > 0 && sound::frame_start(n, fps, rate) > pos {
            n -= 1;
        } else if n < u64::from(u32::MAX) && sound::frame_start(n + 1, fps, rate) <= pos {
            n += 1;
        } else {
            break;
        }
    }
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The system's sound device, through cpal (the `sound-output` feature).
struct Device {
    base: web_time::Instant,
}

impl Device {
    fn new() -> Device {
        Device { base: web_time::Instant::now() }
    }
}

impl Output for Device {
    #[cfg(sound_output)]
    fn open(&mut self, shared: &Arc<Mutex<Shared>>, failed: &Arc<AtomicBool>) -> Result<(Box<dyn Any>, u32), String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host().default_output_device().ok_or_else(|| tr!("no sound output device").to_string())?;
        let supported = device.default_output_config().map_err(|e| tr!("the sound device has no usable format ({error})", error = e))?;
        let config = supported.config();
        let rate = config.sample_rate;
        let base = self.base;
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, config, shared, failed, base),
            cpal::SampleFormat::I16 => build::<i16>(&device, config, shared, failed, base),
            cpal::SampleFormat::U16 => build::<u16>(&device, config, shared, failed, base),
            cpal::SampleFormat::I32 => build::<i32>(&device, config, shared, failed, base),
            other => return Err(tr!("the sound device wants {format} samples, which AniCraft doesn't write yet", format = format!("{other:?}"))),
        }
        .map_err(|e| tr!("could not start sound ({error})", error = e))?;
        stream.play().map_err(|e| tr!("could not start sound ({error})", error = e))?;
        Ok((Box::new(stream), rate))
    }

    #[cfg(not(sound_output))]
    fn open(&mut self, _: &Arc<Mutex<Shared>>, _: &Arc<AtomicBool>) -> Result<(Box<dyn Any>, u32), String> {
        Err(if cfg!(target_arch = "wasm32") { tr!("the web version of AniCraft has no sound output yet").into() } else { tr!("this build of AniCraft has no sound output").into() })
    }

    fn now(&self) -> f64 {
        self.base.elapsed().as_secs_f64()
    }
}

/// An output stream whose callback mixes from the player. The lock is held only while one
/// buffer is mixed; the UI holds it only to change a few fields.
#[cfg(sound_output)]
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(device: &cpal::Device, config: cpal::StreamConfig, shared: &Arc<Mutex<Shared>>, failed: &Arc<AtomicBool>, base: web_time::Instant) -> Result<cpal::Stream, cpal::Error> {
    use cpal::traits::DeviceTrait;
    let channels = usize::from(config.channels);
    let rate = f64::from(config.sample_rate);
    let (shared, failed) = (shared.clone(), failed.clone());
    let mut buf: Vec<f32> = Vec::new();
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            buf.resize(data.len(), 0.0);
            // How long until what is written now is heard: the device's latency.
            let ts = info.timestamp();
            let latency = ts.playback.duration_since(ts.callback).as_secs_f64();
            match shared.lock() {
                Ok(mut s) => {
                    s.player.set_latency((latency * rate) as u64);
                    s.player.fill(&mut buf, channels);
                    s.filled_at = base.elapsed().as_secs_f64();
                }
                Err(_) => buf.fill(0.0),
            }
            for (d, s) in data.iter_mut().zip(&buf) {
                *d = T::from_sample(*s);
            }
        },
        // A rerouted stream (a new default device) or a refused real-time boost still plays;
        // anything else ends it, and the next play opens the device again.
        move |e: cpal::Error| {
            if !matches!(e.kind(), cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::DeviceBusy) {
                failed.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use std::cell::Cell;
    use std::rc::Rc;

    /// What the fake device's stream holds on to.
    type Opened = (Arc<Mutex<Shared>>, Arc<AtomicBool>);

    /// A fake device: the test says when time passes and pulls the buffers itself.
    #[derive(Clone, Default)]
    struct Fake {
        time: Rc<Cell<f64>>,
        /// Refuse to open (no device).
        refuse: Rc<Cell<bool>>,
        opened: Rc<Cell<u32>>,
        rate: Rc<Cell<u32>>,
        shared: Rc<std::cell::RefCell<Option<Opened>>>,
    }

    impl Output for Fake {
        fn open(&mut self, shared: &Arc<Mutex<Shared>>, failed: &Arc<AtomicBool>) -> Result<(Box<dyn Any>, u32), String> {
            if self.refuse.get() {
                return Err("no device".into()); // i18n-exempt: a test's fake device
            }
            self.opened.set(self.opened.get() + 1);
            *self.shared.borrow_mut() = Some((shared.clone(), failed.clone()));
            Ok((Box::new(()), self.rate.get()))
        }
        fn now(&self) -> f64 {
            self.time.get()
        }
    }

    impl Fake {
        fn new(rate: u32) -> Fake {
            Fake { rate: Rc::new(Cell::new(rate)), ..Fake::default() }
        }
        /// The device's callback: one buffer of `frames` stereo frames, `latency` frames deep.
        fn pull(&self, frames: usize, latency: u64) -> Vec<f32> {
            let mut buf = vec![0.0; frames * 2];
            if let Some((shared, _)) = &*self.shared.borrow() {
                let mut s = shared.lock().unwrap();
                s.player.set_latency(latency);
                s.player.fill(&mut buf, 2);
                s.filled_at = self.time.get();
            }
            buf
        }
        fn fail(&self) {
            if let Some((_, failed)) = &*self.shared.borrow() {
                failed.store(true, Ordering::Relaxed);
            }
        }
    }

    /// A document with a constant stream sound (0.25) spanning frames 1–`frames`, `rate` Hz,
    /// at `fps`.
    fn streaming(fps: f32, frames: u32, rate: u32) -> Session {
        let mut s = Session::default();
        let run = |s: &mut Session, id: &str, p: serde_json::Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        run(&mut s, "doc.set_stage", serde_json::json!({"fps": fps}));
        let n = (f64::from(frames) / f64::from(fps) * f64::from(rate)).ceil() as usize;
        let wav = anicraft_audio::wav::encode(rate, 1, &vec![0.25; n], anicraft_audio::wav::Bits::Int16);
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("anicraft-sound-out-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        std::fs::write(&path, wav).unwrap();
        let v = run(&mut s, "file.import_to_library", serde_json::json!({"path": path.display().to_string()}));
        run(&mut s, "sound.place", serde_json::json!({"sound": v["sound"], "frame": 0, "sync": "stream", "extend": true}));
        let _ = std::fs::remove_dir_all(&dir);
        s
    }

    #[test]
    fn frame_at_inverts_the_layouts_own_rounding() {
        for (fps, rate) in [(24.0, 48_000), (29.97, 44_100), (30.0, 44_100), (12.5, 8000), (60.0, 48_000)] {
            for f in [0u64, 1, 2, 7, 99, 1000, 54_321] {
                let at = sound::frame_start(f, fps, rate);
                assert_eq!(u64::from(frame_at(at, fps, rate)), f, "{fps} fps, {rate} Hz, frame {f} starts at {at}");
                if at > 0 {
                    assert_eq!(u64::from(frame_at(at - 1, fps, rate)), f.saturating_sub(1));
                }
            }
        }
        assert_eq!(frame_at(u64::MAX, 24.0, 48_000), u32::MAX, "the end of time is clamped");
        assert_eq!(frame_at(100, f32::NAN, 0), frame_at(100, 24.0, 1));
    }

    /// An hour at 29.97 fps through a 44.1 kHz device with uneven buffers and the UI reading
    /// the clock at uneven times: the playhead is never more than a frame from the sound
    /// being heard, and never drifts.
    #[test]
    fn the_playhead_keeps_time_with_the_sound_over_a_long_play() {
        let (fps, rate) = (29.97_f32, 44_100_u32);
        // A second of sound is enough: the clock runs on while the playhead plays.
        let s = streaming(fps, 30, rate);
        let fake = Fake::new(rate);
        let mut out = SoundOut::with_output(Box::new(fake.clone()));
        assert!(out.play(&s.doc, s.state, 0));
        let latency = 441; // 10 ms
        let mut delivered = 0u64; // frames the device has been handed
        let mut seed = 12345u64;
        let mut worst = 0i64;
        let mut checks = 0;
        while delivered < u64::from(rate) * 3600 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let n = 256 + (seed >> 33) as usize % 768;
            fake.pull(n, latency);
            // The device plays in real time: this buffer starts being heard `latency` later.
            let t0 = (delivered as f64) / f64::from(rate);
            delivered += n as u64;
            // The UI looks at some moment while this buffer plays.
            let into = ((seed >> 20) % n as u64) as f64 / f64::from(rate);
            fake.time.set(t0 + into);
            let truth = (delivered - n as u64) as f64 + into * f64::from(rate) - latency as f64;
            let truth_frame = frame_at(truth.max(0.0) as u64, fps, rate);
            let shown = out.frame(fps).unwrap();
            worst = worst.max((i64::from(shown) - i64::from(truth_frame)).abs());
            checks += 1;
        }
        assert!(worst <= 1, "the playhead strayed {worst} frames from the sound");
        assert!(checks > 100_000);
        // And at the end exactly where an hour of sound is.
        let shown = out.frame(fps).unwrap();
        let expected = (3600.0 * f64::from(fps)) as i64;
        assert!((i64::from(shown) - expected).abs() <= 1, "after an hour: frame {shown}, expected about {expected}");
    }

    #[test]
    fn no_device_is_silent_with_one_notice_and_a_later_device_is_picked_up() {
        let s = streaming(24.0, 48, 48_000);
        let fake = Fake::new(48_000);
        fake.refuse.set(true);
        let mut out = SoundOut::with_output(Box::new(fake.clone()));
        assert!(!out.play(&s.doc, s.state, 0), "no device: no sound");
        assert!(out.error.take().is_some(), "a notice");
        assert!(out.frame(24.0).is_none(), "the playhead runs on the wall clock");
        assert!(!out.play(&s.doc, s.state, 0));
        assert!(out.error.take().is_none(), "said once, not on every press");
        out.scrub(&s.doc, s.state, 3);
        // A device appears; after the retry interval it is used.
        fake.refuse.set(false);
        assert!(!out.play(&s.doc, s.state, 0), "not retried at once");
        fake.time.set(RETRY_SECS + 0.1);
        assert!(out.play(&s.doc, s.state, 0));
        assert_eq!(fake.opened.get(), 1);
    }

    #[test]
    fn a_device_that_goes_away_mid_play_falls_back_then_reopens_at_its_new_rate() {
        let s = streaming(24.0, 48, 48_000);
        let fake = Fake::new(48_000);
        let mut out = SoundOut::with_output(Box::new(fake.clone()));
        assert!(out.play(&s.doc, s.state, 0));
        fake.pull(4800, 0);
        assert!(out.frame(24.0).is_some());
        fake.fail();
        assert!(out.lost());
        assert!(out.frame(24.0).is_none(), "lost: wall clock");
        // The next play reopens the default device, here at a new rate.
        fake.rate.set(44_100);
        let fake2 = fake.clone();
        fake2.time.set(10.0);
        assert!(out.play(&s.doc, s.state, 12));
        assert!(out.error.take().is_some(), "said it went away");
        assert_eq!(out.rate(), 44_100);
        let buf = fake2.pull(2048, 0);
        assert!((buf[2 * 1000] - 0.25).abs() < 1e-3, "plays the layout for the new rate");
        assert_eq!(out.frame(24.0), Some(12));
    }

    #[test]
    fn scrubbing_plays_a_short_slice_and_the_monitor_volume_reaches_the_device() {
        let s = streaming(60.0, 120, 48_000);
        let fake = Fake::new(48_000);
        let mut out = SoundOut::with_output(Box::new(fake.clone()));
        out.set_volume(0.5);
        out.scrub(&s.doc, s.state, 10);
        let buf = fake.pull(4800, 0);
        let heard = buf.chunks(2).filter(|f| f[0] != 0.0).count();
        assert!((2300..=2400).contains(&heard), "a twentieth of a second at 60 fps, not one frame: {heard}");
        assert!((buf[2 * 1000] - 0.125).abs() < 1e-3, "at half volume");
        assert!(!out.is_playing());
    }

    #[test]
    fn looping_keeps_the_clock_and_the_end_lets_event_sounds_play_out() {
        let mut s = streaming(24.0, 24, 48_000);
        s.execute("sound.set", &serde_json::json!({"frame": 0, "sync": "event"})).unwrap();
        let fake = Fake::new(48_000);
        let mut out = SoundOut::with_output(Box::new(fake.clone()));
        assert!(out.play(&s.doc, s.state, 0));
        fake.pull(24_000, 0);
        out.wrap(&s.doc, s.state, 0);
        assert!(out.is_playing());
        assert_eq!(out.frame(24.0), Some(0), "back at the start");
        let buf = fake.pull(2000, 0);
        assert!(buf[2 * 1000] > 0.4, "the first play carries on under the new one");
        out.finish();
        assert!(!out.is_playing() && out.is_sounding(), "events play out after the end");
        out.stop();
        fake.pull(1024, 0);
        assert!(!out.is_sounding(), "Stop silences them");
    }
}
