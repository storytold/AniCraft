# Attribution

Every non-code asset AniCraft ships or embeds, and every published work our code follows
closely, with author, source and licence. AniCraft's own code and original assets are
MIT OR Apache-2.0 (see [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE) and
[`NOTICE`](NOTICE)).

**Rule:** when you add an asset (font, icon, image, sound, sample file, data table) or base code
on a specific published implementation, add a row here *in the same change*. Third-party assets
must be permissively licensed and keep their licence file under [`licenses/`](licenses/).
Never commit proprietary material (Adobe or other vendors' icons, presets, sample files, fonts,
shaders or SDK code).

## Bundled in the app

| Item | Use | Author | Source | Licence |
|---|---|---|---|---|
| Ubuntu Light (via `epaint_default_fonts`) | UI proportional font | Canonical Ltd. / Dalton Maag | <https://design.ubuntu.com/font> | Ubuntu Font Licence 1.0, [`licenses/fonts/UFL-1.0-Ubuntu-font.txt`](licenses/fonts/UFL-1.0-Ubuntu-font.txt) |
| Hack Regular (via `epaint_default_fonts`) | UI monospace font | Source Foundry Authors; Bitstream (Vera) | <https://github.com/source-foundry/Hack> | MIT + Bitstream Vera licence, [`licenses/fonts/LICENSE-Hack.txt`](licenses/fonts/LICENSE-Hack.txt) |
| Noto Emoji (via `epaint_default_fonts`) | Symbol fallback | Google | <https://github.com/googlefonts/noto-emoji> | SIL OFL 1.1, [`licenses/fonts/OFL-1.1-NotoEmoji.txt`](licenses/fonts/OFL-1.1-NotoEmoji.txt) |
| emoji-icon-font (via `epaint_default_fonts`) | Symbol fallback | John Slegers | <https://github.com/jslegers/emoji-icon-font> | MIT, [`licenses/fonts/LICENSE-emoji-icon-font.txt`](licenses/fonts/LICENSE-emoji-icon-font.txt) |
| `assets/icons/anicraft-*.svg` (16 files: align, distribute, mask and masked layer, width tool, camera, brush, eraser, eyedropper, muted sound) | Align panel, layer-type, Width tool, Camera, Brush, Eraser and Eyedropper tool icons, the timeline's mute toggle | AniCraft contributors | Original work, drawn on Lucide's 24-unit grid and stroke style (the muted sound keeps the speaker of Lucide's `volume-2`, ISC, crossed out) | MIT OR Apache-2.0 |
| `crates/svg/fixtures/clips-masks-text.svg` | SVG import test drawing (clip paths, a mask, text) | AniCraft contributors | Original work | MIT OR Apache-2.0 |
| `assets/icons/*.svg` except `anicraft-*` (33 files: tools, transport, layer toggles, folders, guide, image, text, sound, chevrons) | UI icons, stroked by `src/icons.rs` | Lucide Icons and Contributors (some derived from Feather by Cole Bemis) | <https://github.com/lucide-icons/lucide> (`icons/<name>.svg`), copied unchanged from the PhotoCraft sibling project's set (`volume-2.svg` from Lucide 0.512.0) | ISC (Lucide) and MIT (Feather), [`licenses/icons/LICENSE-lucide.txt`](licenses/icons/LICENSE-lucide.txt) |
| `assets/app-icon/` (`anicraft.svg`, `anicraft-small.svg` and every render: PNGs, `anicraft.ico`, `hicolor/`) | App icon (engraved rabbit): window, Dock, taskbar, `anicraft.exe`, Help › About | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for AniCraft, vectorised by `assets/app-icon/trace.py` and rendered by `scripts/icons.sh` | MIT OR Apache-2.0 ([`assets/app-icon/LICENSE.txt`](assets/app-icon/LICENSE.txt)) |
| `packaging/macos/dmg/` (`background.tiff`, `dmg-layout.DS_Store`) | macOS DMG window background and Finder layout | AniCraft contributors | Original work drawn by `packaging/macos/dmg/generate.py` in the app icon's colours; layout approach as in the PhotoCraft sibling project | MIT OR Apache-2.0 |
| `docs/images/` (screenshots and `sample-animation.gif`) | README | AniCraft contributors | Original work: AniCraft's own screenshots (`--screenshot`) and sample animation (`anicraft-cli gif`) | MIT OR Apache-2.0 |
| Library symbol-kind marks, selection handles, timeline marks | UI | AniCraft contributors | Original work, drawn in code (`src/panels.rs`, `src/timeline.rs`, `src/stage.rs`) | MIT OR Apache-2.0 |
| Sample animation (ball, star, sun, hills, sky) | First launch, tests, CLI `--sample` | AniCraft contributors | Original work, generated in code (`crates/doc/src/sample.rs`) | MIT OR Apache-2.0 |
| Interactive sample (menu, Play button, spinner) | File › New from Sample (`which: interactive`), Test Movie tests and screenshots | AniCraft contributors | Original work, generated in code (`crates/doc/src/sample.rs`, `interactive`) | MIT OR Apache-2.0 |
| Colour theme | UI | AniCraft contributors | Original work (`src/theme.rs`) | MIT OR Apache-2.0 |
| `docs/brand/artcraft-logo-white.png` | ArtCraft brand mark (wordmark, light ink for dark backgrounds) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/artcraft-logo-white.svg` | ArtCraft brand mark (wordmark, light ink for dark backgrounds) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/artcraft-logo.png` | ArtCraft brand mark (wordmark, dark ink for light backgrounds) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/artcraft-logo.svg` | ArtCraft brand mark (wordmark, dark ink for light backgrounds) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/artcraft-mark-black.png` | ArtCraft mark (symbol, dark ink for light backgrounds) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/artcraft-mark-black.svg` | ArtCraft mark (symbol, dark ink for light backgrounds) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/artcraft-mark.png` | ArtCraft mark (symbol, color) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/artcraft-mark.svg` | ArtCraft mark (symbol, color) | ArtCraft Team | storytold/craftrules `assets/brand/` | ArtCraft brand terms (`docs/brand/LICENSE-brand.txt`), not open source |
| `docs/brand/LICENSE-brand.txt` | Brand license terms | ArtCraft Team | craftrules `standards/license-files` | — (licence text) |

### Test fixtures

| File | Use | Author | Source | Licence |
|---|---|---|---|---|
| `crates/xfl/tests/corpus/` (12 XFL packages: XML, `.xfl` proxies, a publish-settings file in `scripts/`, `bitmaps/LIBRARY/art/checker.png` and `sunset.jpg`) | XFL import corpus (never shipped in the app) | AniCraft contributors | Original work, written by hand from the public description of the XFL format; the two images made for it (a 16×16 checker drawn in a script, and a 32×16 ramp encoded to JPEG with ffmpeg) | MIT OR Apache-2.0 |
| `crates/xfl/tests/corpus/*/golden/*.png` (20 frames, 160×90) | Golden renders of the corpus | AniCraft contributors | Original work: AniCraft's own CPU renders of the corpus (`UPDATE_GOLDENS=1`) | MIT OR Apache-2.0 |
| `crates/audio/tests/data/tone.ogg`, `tone.mp3` | Decoder tests (never shipped in the app) | AniCraft contributors | Generated for these tests: half a second of a 440 Hz sine made by ffmpeg's `sine` source, encoded with libvorbis and libmp3lame | MIT OR Apache-2.0 |

### Optional build input: craft-fonts (not files in this repo)

Builds made with `CRAFT_FONTS_DIR` (all official releases; ADR 0005) embed these from the shared
[storytold/craft-fonts](https://github.com/storytold/craft-fonts). No font file is committed
here.

| Font (embedded at build time) | Use | Author | Licence |
|---|---|---|---|
| Inter Regular | UI prose (primary) | The Inter Project Authors (Rasmus Andersson) | SIL OFL 1.1, shipped as `OFL-Inter.txt` |
| JetBrains Mono Regular | UI data: timeline, readouts, values, shortcuts (primary) | The JetBrains Mono Project Authors | SIL OFL 1.1, shipped as `OFL-JetBrainsMono.txt` |

No font *files* are committed. Every committed image, icon and sound (the two tiny test sounds)
is listed above; everything else is either embedded by a dependency crate or generated by our
code.

## Algorithms and published techniques

Our implementations are written from the public descriptions below. No third-party code was
copied. Code ported from the sibling project PhotoCraft (same project family and licence, MIT
OR Apache-2.0) is noted in the table.

| Technique | Where | Reference | Notes |
|---|---|---|---|
| Signed-area coverage accumulation rasterizer | `crates/render/src/raster.rs` | Raph Levien, *font-rs* (2016), <https://github.com/raphlinus/font-rs>, Apache-2.0; also stb_truetype v2 (public domain) | Our implementation follows the published accumulate-and-prefix-sum approach. Credited here; Apache-2.0 includes a patent licence. |
| Tile binning with per-row backdrops; per-pixel exact area coverage on the GPU | `crates/gpu/src/binning.rs`, `crates/gpu/src/fine.wgsl` | Raph Levien et al., *Vello* (<https://github.com/linebender/vello>, Apache-2.0 OR MIT) and its design notes; the *sparse strips* rendering work from the same group (2025); the coverage integral is our oracle's (font-rs row above) | Written from the public descriptions; no Vello code or shaders copied (ADR 0019). |
| Adaptive Bézier flattening (Wang's formula) | `crates/geom/src/path.rs` | G.-J. Wang (1984); public mathematical result | |
| Porter–Duff "source over" compositing, premultiplied alpha | `crates/render/src/lib.rs` | Porter & Duff, *Compositing Digital Images*, SIGGRAPH 1984 | Public; standard in every graphics stack. |
| Blend modes (separable modes, premultiplied) | `crates/render/src/lib.rs`, `crates/gpu/src/fine.wgsl` | W3C Compositing and Blending Level 1 | W3C Royalty-Free Patent Policy. |
| Adjust Color filter (brightness, contrast, saturation and hue as colour matrices) | `crates/render/src/filters.rs` | Paul Haeberli, *Matrix Operations for Image Processing* (Silicon Graphics, 1993) | Public description; standard matrix operations. |
| Blur, drop shadow and glow by repeated box blurs | `crates/render/src/filters.rs` | Standard image processing (repeated box filters approach a Gaussian, central limit theorem) | Public; no specific implementation followed. |
| Bevel and Gradient Bevel by offset differences of the blurred opacity; Gradient Glow and Gradient Bevel through a colour ramp | `crates/render/src/filters.rs` | Standard image processing: embossing as the difference of an image offset along the light direction (a directional derivative; e.g. R. C. Gonzalez & R. E. Woods, *Digital Image Processing*); parameters and behaviour from Adobe's public Animate user guide and ActionScript 3.0 reference (`BevelFilter`, `GradientGlowFilter`, `GradientBevelFilter`) | Public descriptions only; no Adobe code or assets (ADR 0025). |
| Bevel and gradient filters in SVG export (`feComposite` arithmetic, `feComponentTransfer` tables) | `crates/render/src/svg.rs` | W3C Filter Effects Module Level 1 / SVG 1.1 filters | W3C Royalty-Free Patent Policy. |
| Timeline playback semantics for actions (stop, play, go to frame or label, next/previous frame and scene; button Up/Over/Down/Hit states; mouse events) | `crates/actions`, `crates/doc/src/player.rs` | Behaviour learned from Adobe's public *ActionScript 3.0 Reference* (`MovieClip`, `SimpleButton`, `MouseEvent`) and the public Animate user guide on buttons, plus observation (ADR 0013, ADR 0024) | Clean room: no Adobe code, SDKs or players; names used nominatively. |
| Cubic-Bézier easing (CSS model) | `crates/doc/src/easing.rs` | W3C CSS Easing Functions Level 1 | W3C Royalty-Free Patent Policy. |
| SVG path data syntax (file format v2) | `crates/geom/src/path.rs` | W3C SVG 1.1 / 2 | W3C Royalty-Free Patent Policy. |
| Marching-squares contour tracing (Paint Bucket regions, Brush outlines) | `crates/geom/src/trace.rs` | Lorensen & Cline (1987); public algorithm | Ported from PhotoCraft's mask tracer (same project family and licence) and reviewed. |
| Chamfer distance transform (3-4) and flood fill | `crates/engine/src/cmds_paint.rs` | G. Borgefors, *Distance transformations in digital images* (1986); public algorithms | Gap closing, and growing fills under their outlines. |
| ZIP archive reading (`.fla`) | `crates/xfl/src/zip.rs` | PKWARE APPNOTE.TXT (published ZIP specification); inflate by `miniz_oxide` (MIT OR Zlib OR Apache-2.0) | Stored and deflate entries; no ZIP64 or encryption. |
| JPEG decoding (imported images) | `crates/engine/src/cmds_file.rs` | `zune-jpeg` (MIT OR Apache-2.0 OR Zlib), already in the dependency tree | Decoded to RGBA, kept as PNG. |
| XFL document structure and shape edges | `crates/xfl/src/` | The publicly documented XFL package structure (`DOMDocument.xml`, `LIBRARY/*.xml`, edge strings) | Clean room: no Animate or third-party converter code (ADR 0009). |
| Full SVG path data and elliptical-arc to Bézier conversion | `crates/geom/src/svg.rs` | W3C SVG 1.1 §8.3 and Appendix F.6 (implementation notes) | W3C Royalty-Free Patent Policy. |
| SVG document import: shapes, styling and inheritance, transforms, viewBox and preserveAspectRatio, paint servers (gradients), `use` | `crates/svg` | W3C SVG 1.1 (§5, §6, §7, §9, §13) and SVG 2; CSS Color Module Level 3 (colour syntax and names) | W3C Royalty-Free Patent Policy. |
| SVG clip paths, masks and text: `clipPath`, `mask`, `text`/`tspan` positioning and text chunks, white-space handling | `crates/svg/src/clip.rs`, `crates/svg/src/text.rs` | W3C SVG 1.1 (§10, §14) and SVG 2 (§11 Text); W3C CSS Masking Module Level 1; CSS Fonts Level 3 (`font-family`, `font-weight`, `font-size` keywords) | W3C Royalty-Free Patent Policy. |
| Exact clipping of filled paths by a convex polygon | `crates/geom/src/clip.rs` | I. E. Sutherland and G. W. Hodgman, "Reentrant Polygon Clipping", CACM 17(1), 1974, extended to Bézier segments (de Casteljau subdivision, roots bracketed by critical points) | Published algorithm; no code taken. |
| Shape morphing (cubic normalization, subdivision to equal segment counts, winding and start-point alignment) | `crates/geom/src/morph.rs` | Standard curve-blending techniques from the public computer-graphics literature (de Casteljau subdivision, exact degree elevation) | |
| Affine QR decomposition for tweening | `crates/geom/src/affine.rs` | Standard linear algebra | |
| Least-squares cubic Bézier curve fitting (Pencil "Smooth", Brush centre lines and outlines; deleting a motion path point refits its neighbours, `Path::merge_at`) | `crates/geom/src/fit.rs`, `crates/geom/src/edit.rs` | P. J. Schneider, *An Algorithm for Automatically Fitting Digitized Curves*, Graphics Gems (1990) | Ported from PhotoCraft's mask tracer and adapted for open strokes. Published algorithm. |
| Monotone cubic interpolation of stroke widths (width profiles) | `crates/doc/src/width.rs` | F. N. Fritsch and R. E. Carlson, *Monotone Piecewise Cubic Interpolation*, SIAM J. Numer. Anal. 17(2), 1980 | Published algorithm (harmonic-mean tangents). |
| Variable-width stroke outlines (trapezoids, outer round joins, caps) | `crates/render/src/stroke.rs` | Standard stroking construction from the public computer-graphics literature | Own implementation (ADR 0018); built-in profiles are AniCraft's own. |
| Brush outlines (the swept tip as a signed distance field: exact distance to the convex hull of two tips, L2 for round and L∞ for square tips, minimised in closed form; traced at level zero) | `crates/geom/src/brush.rs` | Standard sweep and distance-field constructions from the public computer-graphics literature; behaviour of Animate's Brush tool learned from Adobe's public help and observation | Own implementation (ADR 0033); no Adobe code or presets. |
| Eraser (the swept tip as a fill in the planar merge whose regions are dropped; exact fill areas by Green's theorem with five-point Gauss–Legendre quadrature) and Eyedropper | `crates/engine/src/cmds_erase.rs`, `crates/engine/src/cmds_paint.rs` | Green's theorem and Gauss–Legendre quadrature (public mathematical results); behaviour of Animate's Eraser and Eyedropper learned from Adobe's public help and observation | Own implementation (ADR 0034); no Adobe code or presets. |
| WebAssembly plug-in host (module validation, limits, fuel, resumable host calls) | `crates/plugins` | W3C WebAssembly Core Specification 1.0 / 2.0 (W3C Royalty-Free Patent Policy); the `wasmi` interpreter (MIT OR Apache-2.0) | Host model ported from PhotoCraft's filter plug-ins (same project family and licence) and extended with the AniCraft request ABI (ADR 0017). |
| De Casteljau subdivision, Ramer–Douglas–Peucker simplification | `crates/geom/src/edit.rs` | Public mathematical results (de Casteljau 1959; Ramer 1972; Douglas & Peucker 1973) | |
| WAV (RIFF WAVE) reading and writing: PCM 8/16/24/32-bit, IEEE float, `WAVE_FORMAT_EXTENSIBLE` | `crates/audio/src/wav.rs` | Microsoft/IBM, *Multimedia Programming Interface and Data Specifications 1.0* (1991), and Microsoft's public `WAVE_FORMAT_EXTENSIBLE` description | Public file format descriptions. |
| Resampling by cubic Hermite (Catmull-Rom) interpolation | `crates/audio/src/mix.rs` | E. Catmull and R. Rom, *A class of local interpolating splines* (1974); R. Keys, *Cubic convolution interpolation for digital image processing* (1981) | Public mathematical results. |
| Ogg Vorbis decoding | `crates/audio/src/vorbis.rs` | `lewton` (MIT OR Apache-2.0) with `ogg` (BSD-3-Clause); Xiph.Org's open, royalty-free Vorbis I specification | Used as a library, not copied. |
| MP3 (MPEG-1/2 audio layers I–III) decoding | `crates/audio/src/mp3.rs` | `nanomp3` (MIT OR Apache-2.0), a port of minimp3 (CC0) | Used as a library. The MP3 patents have expired; decoding only, no encoder. |

## Rust dependencies

About 420 crates across all target platforms, all permissively licensed. Policy, audit command,
results and elected licences are in [`docs/legal/compliance.md`](docs/legal/compliance.md). The
generated inventory is [`docs/legal/third-party-inventory.txt`](docs/legal/third-party-inventory.txt).
