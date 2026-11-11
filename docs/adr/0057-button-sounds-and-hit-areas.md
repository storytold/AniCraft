# ADR 0057: Sounds on button frames and hit-area editing

* Status: accepted (2026-11-11)
* Relates to: ADR 0024 (the interactive player and Test Movie), ADR 0020 and ADR 0039 (sounds,
  effects, volumes), ADR 0028 (HTML canvas export), ADR 0045 (editing a symbol in place),
  ADR 0055 (9-slice scaling), ADR 0009 (XFL import), ADR 0040 (interface languages)

## Context

Buttons already show Up, Over and Down and answer the pointer through their Hit frame in Test
Movie and the HTML page (ADR 0024, 0028), but a sound on a button's frame never played: the
soundtrack layout skips buttons (they wait for the pointer) and the player only started sounds
of frames that timelines *enter*. Artists coming from Animate put a click on a button's Over or
Down frame and expect to hear it when they point at or press the button. They also expect the
four frames to say what they are, and they need to see the Hit area while drawing the other
states, which Animate leaves to guesswork (the Hit frame is never shown).

Behaviour was learned from Adobe's public help ("add a sound to a button's Over or Down
frame") and from the **SWF File Format Specification** (published by Adobe as an open
specification), whose `DefineButtonSound` tag keeps four sounds by state transition:
`OverUpToIdle`, `IdleToOverUp`, `OverUpToOverDown`, `OverDownToOverUp`. No Adobe code, SDK or
asset was used.

## Decision

1. **When a button sounds (player, `doc::player`).** Each button's pointer state is one of
   `ButtonState::{Up, Over, Down, OutDown}` (pressed and dragged off shows Up). Every change of
   hover, press or release is diffed per button, before its handlers run, and a change plays
   the sound on the frame the SWF transitions name:

   | Change | Frame whose sound plays |
   |---|---|
   | Up → Over (the pointer arrives) | Over (2nd) |
   | Over → Up (it leaves) | Up (1st) |
   | Over → Down (pressed) | Down (3rd) |
   | Down → Over (released over it) | Hit (4th) |
   | dragged off and back while pressed, released off | none |

   Arriving and pressing in one event plays Over's, then Down's. The sound is the keyframe
   *starting* at that frame on every sound-carrying layer of the button (as entering a frame
   does), at the volume of its layer times the layers holding the button (ADR 0039). Start and
   Stop keep their meaning; **a stream plays as an event**, since a button frame has no span to
   stream over. A button shorter than four frames has no sound for the frames it lacks. The
   sounds go out in `Step::sounds` from `input` and `step` alike, so Test Movie (which already
   triggers what each step reports) plays them with no change of its own, deterministically and
   without a device in tests.
2. **The soundtrack is untouched.** `sound::voices` still leaves buttons out: buttons only
   sound interactively. Export Audio, video and Lottie are unchanged (tested: a movie whose only
   sounds are on a button exports silence).
3. **HTML canvas.** The page already carries every symbol's frame sounds (`snd`); `player.js`
   gets the same state diff (`buttonState`, `buttonSoundFrame`, `buttonSounds`) and canvas.js
   plays the result with Web Audio as it plays every other sound. The export no longer reports
   "sounds on button frames". A Boa test plays the same pointer script through both players and
   compares the sounds started, their sync and volume, batch by batch.
4. **One hit area.** `player::hit_frame` (the 4th frame, else Up) and `player::hit_area` (that
   frame's artwork without guides and mask artwork, unclipped) are what the hit test uses, and
   what is drawn: `player::tinted` lays any display list over the picture as **one** group at
   an opacity, every item in one colour (so overlapping shapes don't darken each other), and
   `player::hit_overlay` does it for every button on the stage.
5. **Editing a button in place (ADR 0045).** While a button is open and its playhead is on any
   frame but its hit frame, `Session::stage_picture` adds its hit timeline (mask layers removed
   and what they masked made whole, guide layers hidden) on top, as an instance tinted fully in
   the theme's `hit_area` colour at 35 % (`theme::HIT_AREA_ALPHA`). The tint is view state
   (`Session::hit_area`, set by the app from the theme every frame; `None` headless, so the
   picture is unchanged there) and part of the picture's key, so every tile, GPU or CPU, draws
   the same overlay. The timeline names the four frames **Up, Over, Down, Hit** in the ruler
   and in the playhead's flag (prose, `Type::Caption`, translated with the context "button
   state"), underlines the Hit frame in the overlay's colour, explains each on hover, and widens
   its frames to fit the names while the button is open, restoring the zoom on closing unless
   the artist zoomed meanwhile.
6. **Test Movie: Show hit areas**, a checkbox in the window's bar: every hittable button's
   hit area over the picture (`hit_overlay`), kept across Restart.
7. **Headless.** `control.test_movie` returns `sounds: [{tick, sound, name, sync, gain}]`
   (capped at 10,000, the rest counted in `more_sounds`), so button sounds can be checked from
   the CLI and MCP.
8. **XFL (ADR 0039's rule, now applied).** `DOMSoundItem`s whose file is in the package
   (`LIBRARY/<href>`, or `bin/<soundDataHRef>` when that is a plain WAV, Ogg Vorbis or MP3) come
   across into the Library, in their folders; sounds kept only in Animate's own data format are
   reported ("sounds stored in Animate's own format"), as are missing files. Keyframes keep
   their sound (`soundName`, `soundSync`, `soundLoopMode`/`soundLoop`) on every timeline,
   buttons' included; a frame whose sound didn't come across, sound effects and envelopes, and
   in/out points are reported. **Assumption, not yet verified:** the attribute names
   (`soundName`, `soundSync`, `soundLoopMode`, `soundLoop`, `soundEffect`, `inPoint44`,
   `outPoint44`) follow the publicly described XFL format and have not been checked against a
   file saved by Animate; our test package is self-made.
9. **9-slice guides are remembered** (from the ADR 0055 review). `Symbol::scale_grid_kept`
   (`#[serde(default)]`, left out when empty) holds the guides while 9-slice is off;
   `symbol.set_scale_grid {on: true}` (or moving a guide) brings them back where they were
   instead of starting again from the artwork. Loading drops a kept grid that isn't usable, or
   one kept beside guides that are on. Older files open unchanged.

## Defaults (accepted by the project lead, 2026-11-11)

* **Up and Hit frame sounds play too** (on leaving and on release), as the SWF format keeps
  them, rather than only Over and Down.
* **The hit area is drawn over the artwork**, translucent, rather than under it: it stays
  visible on busy states.
* **The timeline widens a button's frames** for their names rather than abbreviating them.

## Not in this slice

Editing the Hit frame's shape from another state (drawing straight into the overlay), a
separate outline style for the overlay, sounds on movie clips used as buttons (they have no
states), keyboard-driven button presses (Tab focus, Enter), sound envelopes and in/out points
from XFL, and button sounds in Lottie (Lottie has no interactivity).

## Consequences

* Older files open unchanged; files without kept guides are byte-identical.
* The fuzz over every command now also runs with a button open for editing and its overlay
  on, and draws every button's hit area after a pointer goes over and presses.
