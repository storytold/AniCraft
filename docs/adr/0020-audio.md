# ADR 0020: Sounds and audio layers

* Status: accepted (2026-11-09), first slice built
* Relates to: requirements (Audio, P1), roadmap M5, ADR 0003 (file format), ADR 0004
  (destructive actions), `docs/legal/compliance.md`

## Context

Animators import a soundtrack, dialogue or effects, see the waveform on the timeline, animate
to it (lip sync), hear it while playing and scrubbing, and get it in their video. Animate's
model, which our users know: a sound lives in the library and is placed on a **keyframe** of a
layer, with a **sync** mode (Event, Start, Stop, Stream) and **Repeat ×N** or **Loop**.

Constraints: Rust only, no unsafe, never crash on hostile files, permissive licences only
(symphonia, the obvious decoder, is MPL-2.0 and not allowed), no patent-pool encoders in the
binary, time is a function, everything is a command, the CPU path is the oracle, and tests never
open a real audio device.

## Decision

**A new L0 crate, `anicraft-audio`**, with no document knowledge: decoding into `Pcm` (`f32`,
mono or stereo), a pure mixer over `Voice`s (a sound placed in output frames), WAV writing,
waveform `Peaks`, and a device-independent `Player`. Decoders:

| Format | Decoder | Licence |
|---|---|---|
| WAV (PCM 8/16/24/32, float 32/64, extensible; first two channels kept) | our own reader, from the RIFF/WAVE descriptions | ours |
| Ogg Vorbis | `lewton` (+ `ogg`) | MIT OR Apache-2.0 (BSD-3-Clause) |
| MP3 | `nanomp3`, a pure-Rust port of minimp3 | MIT OR Apache-2.0 |

Every decoder is bounded (64 Mi samples, about 12 minutes of stereo) and the third-party ones
run under `catch_unwind`, so a malformed file is an error, never a crash.

**Document model.** `Document.sounds: Vec<Arc<Sound>>` (like bitmaps). A `Sound` keeps **the
bytes it was imported as** (WAV, OGG or MP3, base64 in `.anicraft`) plus its rate, channels and
length, and decodes once on first use into a cache shared by every clone of the document.
`Keyframe.sound: Option<FrameSound { sound, sync, repeat, loop }>`. Both are
`#[serde(default)]` and skipped when empty, so older files open unchanged and files without
sound are byte-identical to before.

**Time is a function.** `doc::sound::voices(doc, rate)` lays the whole movie's sounds out by
playing the timeline from the start: every scene in order, every layer except guides (they are
not exported, so not heard). Event plays in full (Loop: to the movie's end); Start is skipped
while the same sound is playing; Stop cuts it; Stream ends with its keyframe's span (Loop:
loops within it). Frame *f* starts at output frame `round(f · rate / fps)`, computed from the
frame number, so nothing drifts.

**Mixing** (`audio::mix`) is pure: equal rates copy samples exactly (times the gain), other
rates use 4-point cubic Hermite (Catmull-Rom) interpolation with positions computed from the
voice start. A mix also takes an *origin*: event sounds that started before it are silent,
stream sounds are heard from wherever the playhead is. That one rule gives Animate's playback
behaviour (an event sound is heard when its keyframe is reached; a stream resyncs after a
jump) and the export of a frame range.

**Output.** `file.export_audio` mixes in 64 Ki-frame blocks to a 16/24-bit or float WAV at any
rate from 8 to 192 kHz. `file.export_video` writes the range's soundtrack to a temporary 48 kHz
WAV and hands it to the user's ffmpeg with the frames: Opus in WebM (royalty-free), AAC in MP4
(encoded by the user's ffmpeg, never by us).

**Playback** lives in the shell (`src/sound.rs`) on `cpal` (Apache-2.0; WASAPI, CoreAudio,
ALSA), behind the default feature `sound-output`. The device opens on first need (never at
start-up, in `--screenshot` runs or in tests); the audio callback calls `Player::fill`, which
mixes with short ramps at starts and stops. While sound plays, **the playhead follows the audio
clock** (picture keeps time with sound, as Stream sync promises); without sounds, or if the
device fails, playback runs on the wall clock as before. A playhead moved while stopped plays
one frame of the stream sounds there (scrubbing). The document's voices are laid out again
whenever its state changes, so edits are heard at once, even mid-playback.

**Commands.** `file.import_to_library`, Import to Stage for sounds (into the library and onto a
new layer at the playhead, its span as long as the sound so the waveform shows in full),
`sound.place`, `sound.set`, `sound.remove`, `sound.rename`, `sound.delete` (asks first when the
sound is in use: undoable tier of ADR 0004), `file.export_audio` (overwrite asks: irreversible
tier). Copied frames carry the sounds they play into other documents.

## Choices made without the project lead (defaults; easy to change later)

* **Sounds are stored as imported.** OGG and MP3 stay compressed; WAV stays as large as the
  user's file. Re-encoding WAV losslessly (FLAC via permissively licensed crates) would roughly
  halve it; the `format` tag on each sound leaves room to add it without a format break.
* **Import to Stage puts a sound on a new layer** spanning the sound's length (artists want the
  waveform visible to animate to it); Import to Library only adds it; the Library's *Place on
  timeline* attaches it to the active layer's keyframe at the playhead.
* **Stream ends at its keyframe's span**, so the waveform shown is exactly what plays.

## Not in this slice

(Effects, layer volume and mute, and sounds inside symbols came next: ADR 0039.)

Sound effects (fade in/out, left/right, custom envelopes) and per-layer volume or mute; sounds
inside symbols' timelines (only scene timelines are heard); sounds in XFL import; audio in
Lottie or HTML export; lossless compression of stored WAVs; playback latency compensation
(the playhead may lead the sound by one device buffer, about 10 ms); an event sound still
playing when looped playback jumps back to frame 1 is cut there.

## Consequences

* Decoding, mixing and the timing rules are tested sample-exactly without a device; the fuzz
  over every command runs with a sound in the document.
* Linux and BSD builds need the ALSA development package (`libasound2-dev` / `alsa-lib`) for
  `cpal`, or `--no-default-features` for a silent build. CI installs it.
