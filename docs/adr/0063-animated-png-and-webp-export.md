# ADR 0063: Animated PNG and animated WebP export

* Status: accepted (2026-11-11)
* Relates to: ADR 0002 (CPU renderer as the oracle), ADR 0004 (destructive actions), ADR 0017
  (plug-in policy), ADR 0036 (web build), ADR 0040 (interface languages), ADR 0060 (Publish
  Settings)

## Context

AniCraft exports animated GIF, video through the user's ffmpeg, Lottie, an HTML canvas page and
sprite sheets. GIF has 256 colours per frame and 1-bit transparency: gradients band and soft
edges get a hard fringe over any background other than the one they were made on. Animators who
post to the web and chat apps want **full colour with real alpha** in one file that plays
everywhere without a player. Two formats do that and are supported by every current browser:
**APNG** (animated PNG) and **animated WebP**.

The specifications are public: the APNG specification (Mozilla, now part of the W3C PNG
Third Edition) and Google's WebP container and lossless bitstream specifications. Both
formats are royalty-free.

## Decision

1. **Two commands, one pipeline.** `file.export_apng` and `file.export_webp` (File › Export ›
   Export Animated PNG… / Export Animated WebP…) take `path`, `from`, `to` (movie frames, every
   scene in order, as GIF), `scale`, `loop` (`true` = forever, the default; `false` = once; or
   how many times it plays, 0 = forever, at most 65535) and `background` (default **off:
   transparent**, the reason to pick these formats; on: the stage colour). Both live in
   `crates/engine/src/anim.rs`.
2. **Rendering is GIF's.** Frames render on the CPU renderer (the oracle) in parallel, a batch
   at a time. The batch is sized so that at most 32 Mpx are held at once (2 to 16 frames), so
   memory stays bounded at any size. Publish's `Control` counts frames and cancels between
   them. The size limit is GIF's (600 frames of 1280 × 720 in pixels). WebP frames are also
   limited to 16384 px per side.
3. **Repeated frames are stored once.** A frame byte-identical to the one before it lengthens
   that frame's delay and adds nothing else. Holds, which animators use all the time, cost
   nothing.
4. **Only what changed is stored.** Each later frame is cut down to the smallest rectangle
   holding every pixel that differs from the frame before it. Neither format disposes frames
   (dispose op *none*): each frame is drawn over the one before it.
   * **APNG**: when every changed pixel in the rectangle is opaque, the frame **blends over**
     the canvas and its unchanged pixels are written fully transparent. They compress to
     almost nothing, and a transparent pixel blended over leaves the canvas exactly as it was.
     Otherwise (semi-transparent edges in a transparent export) the rectangle **replaces**
     what is there (blend op *source*). Both are exact.
   * **WebP**: rectangles start on even pixels (the format halves offsets) and always replace
     (no blending). The decoders' blend (libwebp's, and `image-webp`'s port of it) rounds an
     opaque pixel's channels down by one, so blending would not be lossless.
5. **Timing is exact.** The frame rate is taken as a fraction p/q (24 → 24/1, 29.97 →
   2997/100, 23.976 → 2997/125).
   * APNG delays are fractions: a frame shown for k frames has a delay of k·q/p s, reduced.
     2 frames at 29.97 fps is 200/2997 s, never a rounded 67 ms.
   * WebP durations are whole milliseconds. Each one is the difference between the rounded
     starts of this frame and the next, so the rounding never adds up and the running time
     is exact to the millisecond.
   * A delay longer than one frame header holds (65535/q frames in APNG, 4.6 hours in WebP)
     continues as extra frames that change nothing: one transparent pixel blended over the
     corner. Timing is never cut short.
6. **Frames compress in parallel; we write the container.** Each frame's rectangle is
   compressed on the thread pool as an image of its own: a PNG from the `png` crate (already
   in the tree; RGBA8, adaptive filters, its best compression), or a lossless WebP from
   `image-webp` (MIT OR Apache-2.0, pure Rust, `#![forbid(unsafe_code)]`). The file is then
   put together in order by hand from the published chunk layouts. For APNG that is `IHDR`,
   `acTL`, `fcTL` with `IDAT` for the first frame (so a plain PNG viewer shows the first
   frame), `fcTL` with `fdAT` for the rest, and `IEND`. For WebP it is `RIFF`/`WEBP`, `VP8X`,
   `ANIM` and one `ANMF` per frame wrapping that frame's `VP8L` chunk.
   * The `png` crate can write APNG itself, but only from raw pixels, one frame after another
     on one thread. Compression is most of the cost, so we use the crate for each frame's
     data and write the chunks (a length, a type, the data and a CRC-32) ourselves.
     `image-webp` has no animation encoder at all.
7. **Lossless WebP only, for now.** There is no permissively licensed, pure-Rust lossy (VP8)
   WebP encoder on crates.io: `image-webp` encodes lossless only, and the `webp` and
   `libwebp-sys` crates bind the C library, which this project does not build. The
   commands therefore have no `quality` parameter yet. When a permissive pure-Rust VP8
   encoder exists, `quality` (lossy, with an `ALPH` chunk for alpha) slots into step 6
   without changing anything else.
8. **Everywhere GIF is.** File › Export lists both. Publish Settings (ADR 0060) gains
   *Animated PNG* (`{name}_animated.png`, since `{name}.png` belongs to the PNG image; `.png`
   or `.apng`) and *Animated WebP* (`{name}.webp`), each with frames, scale, loop forever or a
   play count, and stage colour as background. Older profiles open with both off. Replacing a
   file confirms (ADR 0004). Plug-ins can't call either command, like every `file.*`
   command (ADR 0017). The CLI has `apng` and `webp` beside `gif`. In the web build the file
   becomes a download, as GIF's does (ADR 0036).

## Consequences

* Every frame of both formats decodes back to the CPU renderer's pixels exactly (tests decode
  with the `png` crate's APNG reader, compositing by the APNG rules, and `image-webp`'s
  animation decoder), along with the delays, play counts, repeated-frame merging and the
  rectangles.
* On the sample (48 frames, 1280 × 720) APNG is 2019 KB and WebP 1829 KB, against GIF's
  1464 KB of 256-colour frames; at half size they are 741 KB, 666 KB and 660 KB. Both are
  faster to make than GIF: 0.18 s and 0.13 s against 0.42 s (docs/performance.md).
* Some players clamp very short delays. 240 fps is about 4 ms a frame, and a few browsers
  slow down frames shorter than 10–20 ms, as they do for GIF. This is not worked around.
* `image-webp` is a new dependency. Its licence (MIT OR Apache-2.0) and its dependencies
  (`byteorder-lite`, `quick-error`) are already allowed by `deny.toml`.
