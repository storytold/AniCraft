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
| `assets/icons/anicraft-*.svg` (10 files: align, distribute, mask and masked layer) | Align panel and layer-type icons | AniCraft contributors | Original work, drawn on Lucide's 24-unit grid and stroke style | MIT OR Apache-2.0 |
| `assets/icons/*.svg` except `anicraft-*` (32 files: tools, transport, layer toggles, folders, guide, image, text, chevrons) | UI icons, stroked by `src/icons.rs` | Lucide Icons and Contributors (some derived from Feather by Cole Bemis) | <https://github.com/lucide-icons/lucide> (`icons/<name>.svg`), copied unchanged from the PhotoCraft sibling project's set | ISC (Lucide) and MIT (Feather), [`licenses/icons/LICENSE-lucide.txt`](licenses/icons/LICENSE-lucide.txt) |
| `assets/app-icon/` (`anicraft.svg`, `anicraft-small.svg` and every render: PNGs, `anicraft.ico`, `hicolor/`) | App icon (engraved rabbit): window, Dock, taskbar, `anicraft.exe`, Help › About | Project owner (ArtCraft team) | Original artwork drawn in ArtCraft for AniCraft, vectorised by `assets/app-icon/trace.py` and rendered by `scripts/icons.sh` | MIT OR Apache-2.0 ([`assets/app-icon/LICENSE.txt`](assets/app-icon/LICENSE.txt)) |
| `docs/images/` (screenshots and `sample-animation.gif`) | README | AniCraft contributors | Original work: AniCraft's own screenshots (`--screenshot`) and sample animation (`anicraft-cli gif`) | MIT OR Apache-2.0 |
| Library symbol-kind marks, selection handles, timeline marks | UI | AniCraft contributors | Original work, drawn in code (`src/panels.rs`, `src/timeline.rs`, `src/stage.rs`) | MIT OR Apache-2.0 |
| Sample animation (ball, star, sun, hills, sky) | First launch, tests, CLI `--sample` | AniCraft contributors | Original work, generated in code (`crates/doc/src/sample.rs`) | MIT OR Apache-2.0 |
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

### Optional build input: craft-fonts (not files in this repo)

Builds made with `CRAFT_FONTS_DIR` (all official releases; ADR 0005) embed these from the shared
[storytold/craft-fonts](https://github.com/storytold/craft-fonts). No font file is committed
here.

| Font (embedded at build time) | Use | Author | Licence |
|---|---|---|---|
| Inter Regular | UI prose (primary) | The Inter Project Authors (Rasmus Andersson) | SIL OFL 1.1, shipped as `OFL-Inter.txt` |
| JetBrains Mono Regular | UI data: timeline, readouts, values, shortcuts (primary) | The JetBrains Mono Project Authors | SIL OFL 1.1, shipped as `OFL-JetBrainsMono.txt` |

No sound or font *files* are committed, and every committed image is listed above. Everything
else is either embedded by a dependency crate or generated by our code.

## Algorithms and published techniques

Our implementations are written from the public descriptions below. No third-party code was
copied. Code ported from the sibling project PhotoCraft (same project family and licence, MIT
OR Apache-2.0) is noted in the table.

| Technique | Where | Reference | Notes |
|---|---|---|---|
| Signed-area coverage accumulation rasterizer | `crates/render/src/raster.rs` | Raph Levien, *font-rs* (2016), <https://github.com/raphlinus/font-rs>, Apache-2.0; also stb_truetype v2 (public domain) | Our implementation follows the published accumulate-and-prefix-sum approach. Credited here; Apache-2.0 includes a patent licence. |
| Adaptive Bézier flattening (Wang's formula) | `crates/geom/src/path.rs` | G.-J. Wang (1984); public mathematical result | |
| Porter–Duff "source over" compositing, premultiplied alpha | `crates/render/src/lib.rs` | Porter & Duff, *Compositing Digital Images*, SIGGRAPH 1984 | Public; standard in every graphics stack. |
| Blend modes (separable modes, premultiplied) | `crates/render/src/lib.rs` | W3C Compositing and Blending Level 1 | W3C Royalty-Free Patent Policy. |
| Adjust Color filter (brightness, contrast, saturation and hue as colour matrices) | `crates/render/src/filters.rs` | Paul Haeberli, *Matrix Operations for Image Processing* (Silicon Graphics, 1993) | Public description; standard matrix operations. |
| Blur, drop shadow and glow by repeated box blurs | `crates/render/src/filters.rs` | Standard image processing (repeated box filters approach a Gaussian, central limit theorem) | Public; no specific implementation followed. |
| Cubic-Bézier easing (CSS model) | `crates/doc/src/easing.rs` | W3C CSS Easing Functions Level 1 | W3C Royalty-Free Patent Policy. |
| SVG path data syntax (file format v2) | `crates/geom/src/path.rs` | W3C SVG 1.1 / 2 | W3C Royalty-Free Patent Policy. |
| Marching-squares contour tracing (Paint Bucket regions) | `crates/geom/src/trace.rs` | Lorensen & Cline (1987); public algorithm | Ported from PhotoCraft's mask tracer (same project family and licence) and reviewed. |
| Chamfer distance transform (3-4) and flood fill | `crates/engine/src/cmds_paint.rs` | G. Borgefors, *Distance transformations in digital images* (1986); public algorithms | Gap closing, and growing fills under their outlines. |
| ZIP archive reading (`.fla`) | `crates/xfl/src/zip.rs` | PKWARE APPNOTE.TXT (published ZIP specification); inflate by `miniz_oxide` (MIT OR Zlib OR Apache-2.0) | Stored and deflate entries; no ZIP64 or encryption. |
| JPEG decoding (imported images) | `crates/engine/src/cmds_file.rs` | `zune-jpeg` (MIT OR Apache-2.0 OR Zlib), already in the dependency tree | Decoded to RGBA, kept as PNG. |
| XFL document structure and shape edges | `crates/xfl/src/` | The publicly documented XFL package structure (`DOMDocument.xml`, `LIBRARY/*.xml`, edge strings) | Clean room: no Animate or third-party converter code (ADR 0009). |
| Full SVG path data and elliptical-arc to Bézier conversion | `crates/geom/src/svg.rs` | W3C SVG 1.1 §8.3 and Appendix F.6 (implementation notes) | W3C Royalty-Free Patent Policy. |
| SVG document import: shapes, styling and inheritance, transforms, viewBox and preserveAspectRatio, paint servers (gradients), `use` | `crates/svg` | W3C SVG 1.1 (§5, §6, §7, §9, §13) and SVG 2; CSS Color Module Level 3 (colour syntax and names) | W3C Royalty-Free Patent Policy. |
| Shape morphing (cubic normalization, subdivision to equal segment counts, winding and start-point alignment) | `crates/geom/src/morph.rs` | Standard curve-blending techniques from the public computer-graphics literature (de Casteljau subdivision, exact degree elevation) | |
| Affine QR decomposition for tweening | `crates/geom/src/affine.rs` | Standard linear algebra | |
| Least-squares cubic Bézier curve fitting (Pencil "Smooth") | `crates/geom/src/fit.rs` | P. J. Schneider, *An Algorithm for Automatically Fitting Digitized Curves*, Graphics Gems (1990) | Ported from PhotoCraft's mask tracer and adapted for open strokes. Published algorithm. |
| De Casteljau subdivision, Ramer–Douglas–Peucker simplification | `crates/geom/src/edit.rs` | Public mathematical results (de Casteljau 1959; Ramer 1972; Douglas & Peucker 1973) | |

## Rust dependencies

About 420 crates across all target platforms, all permissively licensed. Policy, audit command,
results and elected licences are in [`docs/legal/compliance.md`](docs/legal/compliance.md). The
generated inventory is [`docs/legal/third-party-inventory.txt`](docs/legal/third-party-inventory.txt).
