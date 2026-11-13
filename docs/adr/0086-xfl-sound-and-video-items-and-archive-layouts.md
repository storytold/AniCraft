# ADR 0086: XFL sound and video items, and the remaining `.fla` archive layouts

* Status: accepted (2026-11-13)
* Relates to: ADR 0009 (XFL import), ADR 0035 (the corpus), ADR 0054 (the zip reader),
  ADR 0039 (sound effects and envelopes), ADR 0057 (sounds on frames and buttons), ADR 0020
  (the audio crate), roadmap M4

## Context

Sound items with their file in the package already came across (ADR 0057), but everything
else about sound was only counted: effects and envelopes were dropped with one line, and a
sound in a format AniCraft can't decode was "1 sounds stored in Animate's own format", with no
name, so an artist couldn't tell which sound to fix. Videos were counted as "videos" and
dropped. And two archive layouts real users hand us were refused: an XFL folder zipped as a
folder (`walk/DOMDocument.xml`, often with macOS's `__MACOSX/` beside it), and a zip with data
in front of it (a self-extracting archive renamed to `.fla`), whose offsets may count from the
zip data rather than the file's start.

Sources, all public: the *SWF File Format Specification* (version 19: `SOUNDINFO`,
`SOUNDENVELOPE`, `SCREENVIDEOPACKET`) and the *Video File Format Specification* (FLV, version
10), both published by Adobe as open specifications; PKWARE's APPNOTE 6.3 for zip; the
publicly described XFL attribute names. No Animate code, SDK or file was used; the corpus is
self-authored.

## Decision

1. **Sound effects come across.** `soundEffect` names a preset as the Effect menu does (case,
   spaces and punctuation ignored): Left Channel, Right Channel, Fade Left to Right, Fade Right
   to Left, Fade In, Fade Out, mapped to `SoundEffect`'s presets. A `SoundEnvelope` with
   `SoundEnvelopePoint`s (`mark44`: samples at 44.1 kHz; `level0` left and `level1` right,
   `0..=32768`, as `SOUNDENVELOPE`) becomes `SoundEffect::Custom`, cleaned by `clean_points`
   (at most 16 points, times and levels clamped, in order), when the effect is `custom`,
   absent or none. A preset name wins over points beside it. An unknown name plays without an
   effect and is counted ("unknown sound effects (played without)"). **In and out points**
   stay reported: the model has no trim, and faking one with an envelope would be wrong for
   repeats; a later slice adds trims to `FrameSound` (and to the Edit Envelope window).
2. **What doesn't come across is named.** `Report::names` (kind → names, at most 64 kept per
   kind; the count stays whole) lists each sound or video that didn't come across with the
   reason; the summary line shows the first three per kind, then "and N more", and the whole
   line is on the status bar's hover. Sounds: "sounds in formats AniCraft can't decode"
   (`theme.aif: AIFF`, `voice.wav: WAV in IMA ADPCM`, `x: Animate's own sound data`, `a damaged
   MP3 file`, FLAC, Ogg Opus, AAC, Sun AU — told from the first bytes; a WAV's codec is read
   from its `fmt ` chunk), "sounds whose files couldn't be read", "missing sound files". The
   kinds' names changed from ADR 0057's ("sounds stored in Animate's own format" is now a
   reason inside "sounds in formats AniCraft can't decode"). `file.open` returns `named` for
   the CLI and MCP.
3. **Videos: a still where we can read one, a name always.** AniCraft has no video playback
   (and won't bundle patent-pool decoders). A `DOMVideoItem`'s data is looked for as
   `LIBRARY/<href>`, the package-relative `sourceExternalFilepath`, then `bin/<videoDataHRef>`.
   If it is an FLV whose first video tag is a **Screen Video** key frame (codec 3: blocks of
   zlib-compressed BGR, public and free of patents), that frame becomes a bitmap, "`<video>`
   (first frame)", in the video's Library folder, and each `DOMVideoInstance` shows it as a
   bitmap rectangle sized to the instance's `frameRight`/`frameBottom` (twips). The report
   still names it ("videos (first frame kept as a still)"), since it no longer plays. Every
   other video is named with its format and length ("intro.flv: FLV, On2 VP6, 4.5 s"; MPEG-4,
   WebM, AVI and Animate's own data told apart by their first bytes; length from `length` in
   seconds, else `frameCount / fps`), and its instances are counted. Sorenson H.263, VP6,
   Screen Video 2 and H.264 are named, not decoded.
4. **A package in one top folder.** When the archive has no root `DOMDocument.xml`, the
   package is the one top folder holding one (`__MACOSX` ignored); reads are prefixed with it.
   Several top folders with documents refuse the archive, naming them ("the archive holds 2
   Animate documents (Run, Walk): unzip it and open one of them") rather than picking one.
   Only one level is looked in: deeper is someone's project folder, not a document.
5. **Data before the archive.** The end record is already found by the bounded backwards search
   (last 64 KB + 22 bytes). If the central directory isn't at the offset the record states but
   a central header sits where the directory must start (right before the end record, or the
   ZIP64 end record), every offset is moved by the difference: the stub's length. A ZIP64 end
   record not at its stated offset is looked for just before its locator. Archives whose
   offsets were fixed to count from the file's start (`zip -A`) need no correction. Nothing
   else is guessed: all of ADR 0054's checks (spans, overlaps, sizes, checksums, ratio) run on
   the corrected offsets, and a stub archive whose directory doesn't end at the end record is
   refused as before.
6. **Bounds.** At most 10,000 video items; FLV tags looked through for the first video: 4,096;
   every tag, block and pixel length checked against its bytes; a Screen Video block inflates
   to exactly its size or the video is named, not shown; images over 8,192 px are refused by
   `Bitmap::from_rgba`. Sound decoding was already bounded by the data's length (a WAV whose
   `data` claims 4 GB reads what is there; `MAX_SAMPLES`), and repeats by `MAX_REPEAT`.

## Assumptions, not yet verified against files saved by Animate

The XFL attribute names `soundEffect` (and its values spelt as the menu), `SoundEnvelope`,
`SoundEnvelopePoint`, `mark44`, `level0`, `level1`, `DOMVideoItem`'s `videoDataHRef`,
`videoType`, `length`, `fps`, `frameCount`, and `DOMVideoInstance`'s `frameRight`/`frameBottom`
follow public descriptions of the format; our packages are self-made (as in ADR 0057). The
embedded video's data file in `bin/` is Animate's own; it is used only if it happens to be an
FLV.

## Testing

* Two corpus packages (ADR 0035): **sounds** (WAV and MP3 that decode, AIFF, IMA ADPCM WAV,
  Animate data, a missing file; stream/loop, event, start with repeats, stop; every preset,
  a custom envelope, an unknown effect, in/out points) and **video** (a 64 × 48 Screen Video
  FLV in 32-pixel blocks with partial top blocks, a VP6 FLV in `bin/`, a missing MPEG-4), each
  with exact report counts **and names**, structural checks (effects, envelope points, sync,
  the still's pixels the right way up, its instance's 1.5× size), golden renders with colour
  probes, `.anicraft` round trips, and the hostile corpus fuzz now garbling sound, video and
  data files too.
* Six new `.fla` layouts packed by the test writer: a top folder with folder entries, a top
  folder with `__MACOSX`, a stub with offsets from the zip data, a stub with offsets from the
  file's start, a stub before a ZIP64 archive, and all of them at once; every corpus package
  imports identically from each, and renders the same goldens. The hostile archive fuzz now
  attacks the top-folder and both stub layouts too: 36,390 archives per run (was 9,664), none panic; the corpus fuzz runs 5,922 hostile packages.
* Unit tests: the Screen Video decoder (block order, partial blocks, every cut and 2,000 random
  byte changes), codecs and containers named, hostile WAVs (every cut, a 4 GB `data` claim, a
  32-channel header with no frames), the summary's "and N more", the top-folder rules.

## Consequences

* An artist sees which sound or video didn't come across, and why, and can convert just that
  one; sound effects play as authored.
* A zipped XFL folder or a self-extracting archive opens like any `.fla`.
* Not in this slice: sound in/out points (needs trims in the model), decoding AIFF and
  compressed WAV (would be straightforward later in `anicraft-audio`), video playback or a
  video layer, Screen Video 2 and later frames, sounds' `linkageClassName`, and verifying the
  attribute names above against a file Animate saved.
