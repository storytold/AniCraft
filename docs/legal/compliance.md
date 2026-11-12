# Legal compliance: licences, dependencies, assets, patents and trademarks

> This is an engineering compliance record, not legal advice. Before the first public release,
> have counsel review it, especially the patent and format sections and the open items at the
> end.

## 1. Our licence

AniCraft is **MIT OR Apache-2.0**, the same as its sibling PhotoCraft (`LICENSE-MIT`,
`LICENSE-APACHE`, `NOTICE`; every crate declares `license.workspace = true`).

* Dual licensing is the Rust-ecosystem norm. It is compatible with GPLv2 users (MIT) and
  provides an **express patent licence and patent-retaliation clause** (Apache-2.0 §3) from
  every contributor.
* Contributions are inbound = outbound (accepted under the same dual licence; see
  `docs/contributing.md` when it lands). A CLA is not needed for this model.
* All crates are `publish = false` until publishing is approved.

## 2. Dependency policy (enforced)

Allowed: MIT, Apache-2.0 (incl. LLVM exception), BSD-2/3-Clause, ISC, Zlib, 0BSD, BSL-1.0,
Unicode-3.0, Unlicense. Font licences (OFL-1.1, Ubuntu-font-1.0) are allowed **only** for
`epaint_default_fonts`. Copyleft (GPL, LGPL, AGPL, MPL, EPL, CDDL, SSPL), "non-commercial",
"source-available" and unknown licences are **not allowed**. Crates.io and the explicitly allowlisted Storytold shared library repository are allowed;
unknown registries and git sources remain denied. The native update checker uses `craft-update`
(MIT OR Apache-2.0), pinned to a commit of `storytold/craft-libs`; its HTTP, verification and
platform installer code are shared with the other Craft apps. A narrowly approved exception
accepts `webpki-root-certs` 1.0.12 under CDLA-Permissive-2.0 for Mozilla certificate data pulled
in by the HTTPS verifier. It applies only to that data crate and version; the code licence
allowlist is unchanged.

```sh
cargo deny check licenses bans sources   # policy in deny.toml; must pass before every commit
cargo deny check advisories              # RustSec security advisories (needs network)
```

### Audit result, 2026-10-28 (Cargo.lock at this commit)

* **licenses ok, bans ok, sources ok**: about 420 crates across Windows, macOS, Linux, Android
  and wasm targets.
* Every crate is permissive. Crates that *offer* a copyleft alternative are used under the
  permissive one:

| Crate | Declared | Elected |
|---|---|---|
| `self_cell` 1.3 | Apache-2.0 OR GPL-2.0-only | Apache-2.0 |
| `r-efi` 5.3 / 6.0 (UEFI target only, never compiled for our platforms) | MIT OR Apache-2.0 OR LGPL-2.1-or-later | MIT |

* Full per-crate list: [`third-party-inventory.txt`](third-party-inventory.txt) (regenerate when
  `Cargo.lock` changes).
* **2026-11-09, WebAssembly plug-ins (ADR 0017)**: `wasmi` 2.0 and its `wasmi_core`,
  `wasmi_ir`, `wasmi_collections` (MIT OR Apache-2.0), `wasmparser` 0.228 (Apache-2.0 WITH
  LLVM-exception, already allowed) and `spin` (MIT). Tests only: `wat` and its
  `wasm-encoder`/`wast` (Apache-2.0 WITH LLVM-exception). No new licence. Plug-ins are the
  user's own files: AniCraft ships none in the binary; the sample plug-ins are source under
  `examples/plugins` (MIT OR Apache-2.0).

### Opening documents on macOS (2026-11-11, ADR 0062)

New crate, from crates.io, macOS builds only: `fmv-macos-events` 0.1.0 (MIT OR Apache-2.0),
pinned with `=`. It registers AppKit's open-documents and quit Apple Event handlers so that
AniCraft's own code stays free of `unsafe`. It was read in full before it was added: about 170
lines, no network or file access. Its `objc2` 0.6, `objc2-app-kit` 0.3 and `objc2-foundation`
0.3 (MIT, Zlib OR Apache-2.0 OR MIT) were already in the tree through wgpu. No new licence. It
declares `rust-version = 1.95`, so the workspace's minimum Rust is now 1.95.
**Audit note:** the crate is built only for `cfg(target_os = "macos")`, is pinned with `=0.1.0`
(the checksum is in `Cargo.lock`), and its licence is checked by `cargo deny`. It registers
two Apple Event handlers (open documents, quit) and an observer, and returns paths through a
main-thread inbox. It has no network, file or process access, and no build script. Read any
new version in full before it comes in.

### The user guide (2026-11-11, ADR 0069)

New crates, from crates.io: `pulldown-cmark` 0.13 (MIT) with `pulldown-cmark-escape` (MIT),
`unicase` (MIT OR Apache-2.0), `bitflags` and `memchr`, already in the tree. Default features
off (no command-line binary). It turns the user guide's Markdown into the bundled HTML book. The
guide's screenshots are original work (ATTRIBUTION.md). The guide names Adobe and Animate only
nominatively, with the non-affiliation line on every page. No new licence.

### Find and Replace (2026-11-11, ADR 0058)

New crates, from crates.io: `regex` 1 with `regex-syntax` and `regex-automata` (MIT OR
Apache-2.0) and `aho-corasick` (Unlicense OR MIT), built with only the `std`, `perf`,
`unicode-case` and `unicode-perl` features. They match Find and Replace's patterns in linear
time with size limits. No new licence.

### System contrast setting (2026-11-11, ADR 0064)

New direct dependency on Windows only: `winreg` 0.55 (MIT, crates.io), already in the tree
through `craft-update`. It reads one registry value, the high-contrast flag. The web build
turns on web-sys's `MediaQueryList` feature (same crate, same licence). macOS and Linux ask
system tools (`defaults`, `gdbus`, `gsettings`), so nothing is linked. No new licence.

### Interface languages (2026-11-10, ADR 0040)

New crate: `sys-locale` 0.3 (MIT OR Apache-2.0, crates.io), which reads the system's interface
language (on the web through `js-sys`/`web-sys`, already in the tree). No new licence.
Translations are **not** a dependency: they come from storytold/craft-i18n (MIT OR Apache-2.0)
as an optional build input (`CRAFT_I18N_DIR`), embedded at build time like craft-fonts; no
catalog file is committed here. Its catalogs follow a clean-room rule (written from the plain
meaning of the English, never from another product's localisation), which `docs/localisation.md`
repeats for translators. The plural rules are written from the public Unicode CLDR rules
(`ATTRIBUTION.md`).

### Web build (2026-11-09, ADR 0036)

New crates, all MIT OR Apache-2.0, from crates.io: `web-time` (clocks that work in a browser;
already in the tree through winit), and in the web build only `wasm-bindgen`,
`wasm-bindgen-futures`, `js-sys` and `web-sys` (already in the tree under eframe's web backend).
Tests only, in the WebAssembly build of the engine: `wasm-bindgen-test`, its macro and shared
crates, `minicov`, `cast` and `oorandom` (MIT OR Apache-2.0, `oorandom` MIT), and `nu-ansi-term`
(MIT). No new licence. Build tool, not a dependency: the `wasm-bindgen` CLI (MIT OR
Apache-2.0), installed with `cargo install`. Nothing generated by it is committed. The web page
(`packaging/web/index.html`) is our own and uses no npm packages. The web editor bundles the same
fonts as the desktop app (egui's, plus Inter and JetBrains Mono when built with craft-fonts).

### HTML canvas export (2026-11-09, ADR 0028)

Tests only: `boa_engine` 0.21 (a JavaScript engine in pure Rust, Unlicense OR MIT) and its tree
(`boa_*`, `icu_*` and `zerovec`-family crates under Unicode-3.0 or MIT/Apache-2.0, `ryu-js`
Apache-2.0 OR BSL-1.0, `regress`, `intrusive-collections`, `num-bigint`, `time` and others
under MIT/Apache-2.0), as a dev-dependency of `anicraft-render`: the tests run the exported
page's player in it. Nothing of it is in the app or the CLI. No new licence. The page's
JavaScript is our own code (`crates/render/src/html/`), MIT OR Apache-2.0 like the rest, and
uses no npm packages.

### Animated PNG and WebP (2026-11-11, ADR 0063)

New crate, from crates.io: `image-webp` 0.2 (lossless WebP encoding and decoding in pure Rust,
`#![forbid(unsafe_code)]`, MIT OR Apache-2.0). Its dependencies, `byteorder-lite` (Unlicense
OR MIT) and `quick-error` (MIT OR Apache-2.0), were already in the tree. No new licence. APNG frames use the `png` crate already in
the tree. There is no lossy WebP yet: the only lossy encoders on crates.io bind the C libwebp
library, which this project does not build. APNG and WebP are both royalty-free (see the
codec table).

### Sound (2026-11-09, ADR 0020)

The audio slice adds, all permissive and from crates.io: `lewton` (Ogg Vorbis decoding, MIT OR
Apache-2.0) with `ogg` (BSD-3-Clause), `nanomp3` and `nanomp3-core` (MP3 decoding, MIT OR
Apache-2.0), and in the desktop app `cpal` (sound output, Apache-2.0) with its platform crates
(`windows` on Windows, `alsa`/`alsa-sys` on Linux and the BSDs, MIT OR Apache-2.0; CoreAudio
bindings on macOS). No new licence. `symphonia`, the usual Rust decoder, is MPL-2.0 and was not
used.

### Installers (2026-11-09, ADR 0026)

No new crate. The installers are built with tools that are not Rust dependencies, so
`cargo deny` doesn't see them; they are listed here:

* **WiX Toolset 5** (Windows MSI), Microsoft Reciprocal Licence (MS-RL). A build tool, but the
  MSI also carries parts of it: the WixUI dialog definitions and, for the Finish page's "Launch
  AniCraft", the `Wix4UtilCA` custom-action DLL. MS-RL asks that files containing its code be
  distributed with that licence and their source, which is public at
  <https://github.com/wixtoolset/wix>. Every Craft app ships the same; **counsel to confirm** (open
  item below).
* **AppImage type2-runtime** (prepended to each AppImage), MIT, statically linking libfuse3
  (LGPL-2.1) and squashfuse (BSD-2-Clause). A separate program in the same file, not linked
  with AniCraft. Pinned by version and SHA-256 in `packaging/linux/package.sh`.
* **appimagetool** (MIT), **nfpm** (MIT), **flatpak-builder** (LGPL-2.1), Apple's `hdiutil`,
  `codesign`, `notarytool`, `sips` and `iconutil`, Microsoft's `signtool`: build tools only;
  nothing of them ships.
* The Flatpak runs on the freedesktop runtime from Flathub; the runtime isn't in our bundle.
* `packaging/macos/dmg/generate.py` uses Pillow (MIT-CMU), `ds_store` and `mac_alias` (MIT) to
  write committed files; none of them ships.

## 3. Fonts, graphics and other assets

* **Fonts**: only the four fonts embedded by egui (`epaint_default_fonts`): Ubuntu Light
  (Ubuntu Font Licence 1.0), Hack (MIT + Bitstream Vera), Noto Emoji (OFL 1.1) and
  emoji-icon-font (MIT). All permit embedding and redistribution in commercial and
  non-commercial software. Restrictions apply only to *modifying and renaming the fonts*,
  which we do not do. Their licence texts ship in `licenses/fonts/`.
  * Primary UI typefaces (ADR 0005): Inter and JetBrains Mono, SIL OFL 1.1, embedded from the
    shared `craft-fonts` build input (`CRAFT_FONTS_DIR`), as in PhotoCraft. Font rule: fonts must
    be permissive in use (OFL, MIT, Apache, UFL-style), bundled rather than relying on system
    fonts, and small (≤ 2.5 MB embedded in total). **No font files are committed to this
    repository.**
* **User-selected system fonts** (Preferences › Appearance) are read at runtime from the user's
  own OS installation and never bundled or redistributed by AniCraft, so their licences do not
  bind our distribution. Enumeration uses `fontdb` (MIT).
* **Platform crates** added for ADR 0006: `rfd` (MIT, native file dialogs), eframe's `glow`
  OpenGL fallback (`glow`, `glutin`: MIT OR Apache-2.0). The `directories` crate was rejected
  because its dependency `option-ext` is MPL-2.0; the settings folder is resolved in our own
  code.
* **Icons and UI graphics**: drawn in code by AniCraft contributors (no icon packs, no
  vendor-derived glyphs).
* **Sample content**: generated in code. Screenshots in docs contain only our UI and sample.
* **Never** commit Adobe material: icons, workspace layouts copied pixel-for-pixel, preset
  files, sample `.fla`, brushes or fonts.

## 4. Patents and file formats

| Area | Assessment | Policy |
|---|---|---|
| Rendering algorithms (coverage accumulation, Bézier flattening, Porter–Duff, gradients) | Decades-old published techniques. The coverage approach is from Apache-2.0 open source (font-rs), whose licence includes a patent grant. | Implement from public papers and permissively licensed references only; credit them in `ATTRIBUTION.md`. |
| Animation features (keyframes, classic tweens, easing, onion skin, symbols) | Long-standing prior art in animation software since the 1990s. | No feature copies a specific proprietary implementation. |
| GPU renderer (M3) | **As built** (ADR 0019): tile binning with per-row backdrops and per-pixel exact area coverage, written from the public descriptions of Vello and the sparse-strips work (Apache-2.0 OR MIT) and our own oracle's coverage integral; no third-party shader or code copied. Uses `wgpu` (MIT OR Apache-2.0), `pollster` and `bytemuck`, all already in the tree through eframe; no new licence. | Same rule: public papers and permissive code only. |
| PNG, GIF, SVG, WebP | Royalty-free (GIF's LZW patents expired in 2003–2004). | OK to implement. |
| **Video export** | H.264/HEVC/AAC are covered by patent pools. VP9, AV1 and Opus are royalty-free. | **Never bundle encoders for patent-pool codecs.** MP4/H.264 only by piping to a user-installed ffmpeg. Default to WebM (VP9/AV1). **As built** (`file.export_video`): AniCraft starts the user's ffmpeg (found via a parameter, `ANICRAFT_FFMPEG` or the PATH) with an argument list, never a shell, writes raw RGBA frames to its stdin, and ships no codec code; the default file type is `.webm` (VP9). |
| **Sound** (M5, ADR 0020) | WAV/PCM is unencumbered; Vorbis and Opus are royalty-free; the MP3 patents expired in 2017; AAC is in a patent pool. | Decode WAV, Ogg Vorbis and MP3 in-process (pure-Rust, permissive decoders; no encoders). The video soundtrack is encoded by the user's ffmpeg: Opus for WebM (the default), AAC only for MP4, never by code we ship. Exported audio is uncompressed WAV. |
| **XFL / FLA import** (M4) | Adobe's proprietary formats, without a public specification. | Clean room: work only from files users or we author, public community docs and observation. Never use Adobe code, SDKs or decompiled software. Interoperability-focused (generally protected in EU Software Directive art. 6 and US DMCA §1201(f)); **confirm with counsel before shipping**. |
| **ActionScript compatibility** (M6, ADR 0013) | A programming language; no known patent pool. Implemented from ECMA-262 (3rd edition) and Adobe's public ActionScript 3.0 references. | Clean room as for XFL: public documents and self-authored files only; never Adobe compilers, SDKs or player code. Keep and run a timeline subset only; no bytecode VM. Tier 1 (keeping scripts, built) reads only the XFL `<Actionscript>` element, linkage attributes and publish settings from self-authored files. |
| WebAssembly (plug-ins, ADR 0017) | W3C standard under the W3C Royalty-Free Patent Policy. | Allowed. Interpreted by `wasmi` (no code generation). |
| SWF export (optional) | Adobe published the SWF specification and lifted its use restrictions in 2009 (Open Screen Project). | Allowed, low priority. |
| Lottie | Open specification (Lottie Animation Community, Linux Foundation). | Allowed. |
| HTML canvas export (ADR 0028) | HTML, the canvas 2D context and Web Audio are W3C/WHATWG standards under royalty-free policies. | Allowed. The page's player and drawing code are our own, written from those specifications and our own Rust player; nothing taken from Adobe's CreateJS runtime or other publishers' output. |

## 5. Trademarks

* "Adobe", "Animate" and "Flash" are Adobe trademarks. We use them only *nominatively* in docs
  ("compatible with…", "competitor to…") and in the non-affiliation disclaimer of the legal
  notices (`NOTICE`, Help › Licenses & Notices). Never in the product name, feature UI, icons
  or marketing imagery, and never implying endorsement.
* "ActionScript" is also an Adobe trademark. Docs and compatibility messages (import reports,
  the language a kept script is written in) name it nominatively; the features themselves have
  generic names (Actions panel, frame scripts, Test Movie). See ADR 0013.
* "AniCraft": the project lead accepted the name (2026-11-09). No formal clearance search was
  done; that was their decision.

## 6. Release checklist (before any public binary)

1. `cargo deny check licenses bans sources advisories` passes.
2. Generate the full third-party licence texts for the shipped binary (for example with
   `cargo about generate`; the tool is not yet installed here) and ship them as
   `THIRD_PARTY_LICENSES.html` next to `LICENSE-*`, `NOTICE` and `licenses/`.
3. Help › Licenses & Notices in the app shows the same information (already present).
4. Re-check `ATTRIBUTION.md` against every asset in the build.
5. Counsel sign-off on the open items.

## 7. Open items

| Item | Status |
|---|---|
| Counsel review of this document (patents, XFL interoperability, ActionScript compatibility and naming) | Open, before the first public release |
| "AniCraft" trademark clearance | Settled by the project lead (2026-11-09): the name is kept |
| Freedom-to-operate patent search for core animation features | Open (recommended before 1.0) |
| Full licence-text bundle generation (`cargo about`) in the release pipeline | Open (M8; the installers ship `LICENSE-*`, `NOTICE`, `ATTRIBUTION.md` and the font OFLs today) |
| WiX's MS-RL parts inside the MSI (WixUI, `Wix4UtilCA`) and the AppImage runtime's LGPL libfuse3 | Open: family-wide practice, counsel to confirm (ADR 0026) |
