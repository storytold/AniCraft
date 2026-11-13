# ADR 0039: Sound effects, layer volume and sounds inside symbols

* Status: accepted (2026-11-10), second sound slice built
* Relates to: ADR 0020 (sounds and audio layers), ADR 0024 (the interactive player and Test
  Movie), ADR 0028 (HTML canvas export), ADR 0003 (file format), ADR 0022 (accessibility)

## Context

The first sound slice (ADR 0020) played sounds on scene timelines only, at full volume. Artists
coming from Animate expect three more things:

* the **Effect** menu on a frame sound (Left channel, Right channel, Fade left to right, Fade
  right to left, Fade in, Fade out, Custom) and its envelope editor, where each channel's
  volume is a line through a few points over the sound's time;
* a quick way to **mute** a dialogue or music layer while animating, and to set how loud it is;
* **sounds inside symbols**: lip sync in a graphic symbol, a footstep inside a walk-cycle
  movie clip.

Constraints: time is a function, the mix stays pure and sample-exact, everything is a
command, never crash on hostile files, older files open unchanged.

## Decision

**Effect model.** `FrameSound.effect: SoundEffect`, one of the six presets, `None`, or
`Custom(Vec<EnvelopePoint { at, left, right }>)`: `at` in **seconds from the sound's start**
(repeats included; rate-independent, so swapping the sound or exporting at another rate keeps
the shape), levels `0..=1`. At most 16 points; loaded or commanded points are cleaned (sorted,
clamped). Serialized by name (`"effect": "fade_in"`, `{"custom": [...]}`), skipped when None.
`FrameSound` stops being `Copy`.

**Presets are shaped over the time the sound is heard**, following Adobe's public description
("Fade In gradually increases the volume over its duration"): one play times the repeat count;
a stream's span (or one play times repeats, if shorter); for an event loop, the rest of the
movie. Left and Right channel are a single point (1, 0) or (0, 1). The envelope editor starts
Custom from the preset's points over that same time, so editing a preset changes nothing
until a point moves.

**Mixing.** `audio::Voice` gains `envelope: Option<Arc<Envelope>>` (points in output frames,
straight lines between them, level before the first and after the last) and `skip` (output
frames of the sound already gone by when it is first heard). The mixer multiplies each output
frame's left and right sample by the envelope's gains at the sound's time (window start + skip
+ k), computed in double precision; equal-rate mixing stays a copy times a gain. Tests check
every preset, custom points, repeats, skips and resampling at sample positions against the
same formula.

**Layer volume and mute.** `Layer.volume` (`0..=1`, default 1, skipped when 1) and
`Layer.muted`. The gain of a voice is the product of the volumes of the layers holding it,
from the scene down through every symbol instance; mute is gain 0. A muted layer still
"plays" (its Start and Stop still act on other layers' voices), so muting never changes what
else is heard. Mute applies to exports too: what you hear is what you get, as in a DAW
(Animate has no layer mute to follow).

**Sounds inside symbols.** `voices` walks *stretches*: a timeline shown over movie frames
`g0..`, advancing one frame per frame from frame `l0`, or holding `l0`, and whether `l0` is
*entered* there. The scene is one advancing stretch. On each layer, keyframes that the stretch
reaches give their sound (event, start and stop when entered; streams over the part of their
span the stretch plays, joined mid-way with a skip), and symbol instances on them give child
stretches:

* a **graphic** keeps step with its parent: Loop wraps (each wrap enters frame 0 again), Play
  Once runs to its last frame and holds it (not re-entered), Single Frame holds; a new keyframe
  holding it starts it again from its first frame, as the renderer does;
* a **movie clip** loops from where the instance appeared, carrying on across keyframes that
  hold the same instance (as `eval` draws it on the stage), so a stream in it continues
  sample-exactly across them;
* **buttons** wait for the pointer: not in this slice.

The walk only enters symbols that hold sounds (directly or deeper), stops at the symbol depth
limit, and has a budget of 100 000 stretches, so a one-frame graphic looping for hours inside
another is bounded work. Sync rules (Start skips a playing sound, Stop cuts it) then run over
all voices in time order, as before.

**The interactive player and Test Movie.** `player::Step` reports `sounds: Vec<Started>` (the
frame sound, its gain, its span) for every keyframe entered: the main timeline's and movie
clips' as they enter frames (also after go-tos), and graphics' when the frame a graphic shows
differs from the one it showed when the step began (or it is new). `doc::sound::trigger` turns
those into voices at the step's output frame with the same sync rules; an event loop has no end
in a movie that goes round, so its fade lasts one play. Test Movie feeds them to the audio
player and, while sound plays, steps the movie to the audio clock (as stage playback does);
pausing stops the sound, resuming plays from the movie's frame.

**HTML canvas.** Sound entries carry the layer's gain and the envelope's points in seconds;
layers carry `sg` (their gain) so the page multiplies volumes down through symbols like the
Rust player; the page's player reports graphics' sounds the same way; canvas.js plays an
envelope with one Web Audio `GainNode` per channel (`setValueAtTime` + `linearRampToValueAtTime`)
after a speakers-interpretation up-mix to stereo. A test checks the page's player starts the
same sounds at the same volumes as the Rust player, frame by frame. Sounds on button frames are
reported as not played.

**XFL.** Not imported. Animate keeps a sound's data in the document's `bin/` folder in a form
no public description covers, so sounds (and the envelopes on their frames) cannot come
across from XFL in a clean-room way; the XFL importer already reports sounds as unsupported.

**Commands.** `sound.set` and `sound.place` take `effect` (a preset name, or `custom`, which
keeps or starts points) and `envelope` (1–16 points, makes it Custom); `layer.set_volume
{volume?, muted?}`; `layer.toggle_mute`. All undoable; a dragged point is one undo step.

**Shell.** Properties › Sound: an Effect menu with **Edit…** beside it, and, on layers holding
sounds, a Volume field and Mute. Edit Envelope is a window (not modal) with two lanes, the
channels' waveforms (per-channel peaks, computed once per sound), the volume lines and square
handles; click adds, drag moves (between neighbours, so points keep their order; the time is
shared by both channels as in Animate, the level is the lane's), right-click deletes; the
selected point's time (seconds or frames) and levels are fields, so the editor works from the
keyboard; zoom, fit and a time ruler; Play previews the sound with its envelope. Every handle
and lane has a role and name for screen readers. The timeline row of a layer with sounds has a
speaker toggle (crossed out when muted, an icon drawn on Lucide's speaker) and the waveform is
scaled by the envelope and faint when muted.

## Choices made without the project lead (defaults; easy to change later)

* **Fades last the whole heard time** (Adobe's wording), not a fixed time.
* **Mute is in exports.** A muted layer is silent in Export Audio, video and HTML. The
  alternative (mute as a monitor-only switch) would make an export differ from what is heard.
* **Layer volume tops out at 100 %** (no boost), matching the envelope's range and avoiding
  clipping surprises.
* **Graphics in Test Movie enter a frame when their shown frame changes**, so a graphic held
  on one frame does not repeat its sound.

## Not in this slice

Sounds on button frames (Over, Down); panning other than through the envelope; sounds from
XFL; audio in Lottie; stream sounds that follow go-tos in the HTML page; a stream in a graphic
that starts mid-way in Test Movie (the player starts streams at their keyframe); an envelope
editor playhead during preview.

## Consequences

* Older files open unchanged (every new field is `#[serde(default)]` and skipped at its
  default); files without effects or volumes are byte-identical to before.
* The fuzz over every command runs with fades, custom envelopes and streams inside every
  symbol, and lays out and mixes the result after each run.

## Update (2026-11-11, ADR 0057)

Sounds on button frames now play in Test Movie and the HTML page as the pointer changes a
button's state (Over's going over it, Down's pressing it, Up's leaving, Hit's letting go over
it), and the soundtrack still leaves buttons out. XFL import brings sounds whose file is in the
package (WAV, Ogg Vorbis, MP3), with the sounds on frames that use them; sounds kept only in
Animate's own data format are still reported, as this ADR decided.
