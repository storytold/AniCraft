# ADR 0095: Audio output and sound polish

* Status: accepted (2026-11-13)
* Relates to: ADR 0020 (sounds and audio layers), ADR 0039 (effects, layer volume, sounds in
  symbols), ADR 0036 (web build), ADR 0004 (destructive actions), roadmap M5

## Context

ADR 0020 put sound on the stage through `cpal`: the pure mixer (`audio::mix`) feeds a
device-independent `Player`, the playhead follows the player's clock, and a moved playhead plays
one frame of the streams. What was missing, checked against how Animate behaves and what artists
need when timing to sound:

* the playhead followed the frame being **mixed**, which is one device buffer plus the device's
  latency ahead of the frame being **heard** (tens of milliseconds: a frame at 24 fps);
* an **event sound was cut** when looped playback went round to frame 1 or the playhead stopped
  on the last frame; in Animate event sounds play to their end and only stopping playback
  silences them;
* no way to turn the **listening level** down or mute it while working (only the document's own
  layer volumes, which change the export);
* **scrubbing** always on, and one frame at 60 fps (17 ms) is too short to hear;
* a stream that **failed** (device unplugged) stayed failed until restart, without saying so
  beyond the first open; no device at all was said once and never retried;
* the **web build** was silent;
* no test exercised the shell's clock: it needed a device.

## Decision

**The player** (`anicraft-audio`, L0, still pure):

* **Tails.** `Player::wrap(from)` (a loop going round) and `Player::finish()` (the playhead
  stopped by itself) move every event voice sounding at the playhead onto a second clock as a
  *tail*: the same voice re-based (`at` now, `skip` advanced by what was heard), so its
  envelope, repeats and resampling carry on sample-exactly. At most 64 tails (oldest dropped).
  `play` (a new start) clears them; `stop` (the user stopping) fades everything out.
* **Monitor volume.** `set_volume(0..=1)`, glided over the 256-frame ramp so dragging never
  clicks. It is applied after the mix, so exports (which call `mix` directly) never hear it.
* **The heard clock.** The callback tells the player the device's latency (cpal's
  `playback − callback` timestamps); `heard(since_fill)` is the start of the last buffer, plus the
  wall time since it was filled (never past the buffer), less the latency, never before the play
  started. The shell maps it to a frame with `frame_at`, the exact inverse of the layout's own
  `frame_start` rounding, so picture and the voices' layout agree to the frame.
* Hostile input is clamped: non-finite or out-of-range volumes, latencies and envelopes; the
  resampler's neighbour indices saturate (a fuzz found an overflow on a huge `skip`).

**The shell** (`src/sound.rs`): the device sits behind an `Output` trait (`open`, `now`). The
real one is cpal's default device; tests use a fake that the test pulls buffers from and whose
clock it sets, so no CI run needs a device. On open failure or a stream error other than a
reroute, sound goes silent, the reason is said **once** (status bar notice and an
`anicraft: sound:` line on stderr), playback carries on on the wall clock, and the next play
tries again (at most every 2 s); a new rate re-lays the voices. A rerouted default device
(cpal's `DeviceChanged`) just keeps playing.

**Preferences, not document:** `sound_volume`, `sound_muted` and `audio_scrubbing` are the
listener's, kept in preferences (`#[serde(default)]`, so older preference files load). They are
in the timeline header (a speaker that mutes and a short slider, beside Loop Playback), in
**Control › Mute Sounds** and **Control › Audio Scrubbing** (where Animate users look), and in
**Preferences › Sound**. Like Loop Playback they are view state, so they are not commands; the
document's sound (layer volume and mute) remains commands as before.

**Scrubbing** plays a slice of at least 1/20 s (one frame at 24 fps is already longer).

**Web:** the same `cpal` with its `wasm-bindgen` feature (Web Audio, MIT OR Apache-2.0 glue
already in the web build). The AudioContext opens on the first play, which is a user gesture,
as browsers require. No other web code changes.

**Export** needed nothing new: Export Audio (WAV of the mix) and Export Video (soundtrack through
the user's ffmpeg) already carry sound; GIF, APNG, WebP, SVG and Lottie don't carry audio.

## Choices made without the project lead

* Event sounds keep playing after the playhead stops on the last frame without Loop; Animate
  does the same. Pressing Stop silences them.
* The monitor volume also applies to Test Movie and Library previews (they are listening too).
* The volume control is always in the header, not only when the document has sounds, so the
  header doesn't change shape on import (it still fits at the default window size; a test
  checks).

## Not in this slice

Choosing an output device other than the system default; `stopAllSounds` as a timeline action
(Stop sync covers one sound); a Stop sync sound silencing a tail of the same sound carried over
a loop; per-buffer clock interpolation finer than the wall clock on platforms whose timestamps
are coarse.

## Consequences

* Tests: tails across a loop and after the end, Stop silencing them, the volume glide, the heard
  clock, a hostile-voice fuzz (player), a hostile-envelope fuzz through `sound.set` (engine), and
  in the shell, with the fake device: an hour at 29.97 fps through 44.1 kHz with uneven buffers
  and 10 ms latency stays within one frame of the sound heard at every one of the >100 000
  checks and at the end; no device, a device that appears later, one that goes away mid-play
  and comes back at a new rate; scrub length and volume.
* The web build now links cpal's Web Audio host.
