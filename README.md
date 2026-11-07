<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/brand/artcraft-logo-white.svg">
      <img alt="ArtCraft" src="docs/brand/artcraft-logo.svg" width="200">
    </picture>
  </a>
</p>

<h1 align="center">AniCraft</h1>

<p align="center">
  <b>2D vector animation studio; an open-source, clean-room reimplementation of Adobe Animate, rebuilt in pure Rust.</b>
</p>

<p align="center">
  A native 2D vector animation editor with timeline, layers, keyframes, classic and shape tweens,
  onion skinning, and an egui stage. It runs natively on macOS, Windows, Linux and FreeBSD,
  and in the browser via WebAssembly.<br>
  <i>By the ArtCraft team.</i>
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="egui" src="https://img.shields.io/badge/UI-egui-14a9c4?style=flat-square">
  <img alt="macOS · Windows · Linux · FreeBSD · Web" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-native-2f7bf5?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="https://getartcraft.com/apps/anicraft"><b>AniCraft on getartcraft.com</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">All Crafting Apps</a>
</p>

<br>

> [!NOTE]
> **ArtCraft is a community of artists from all walks of life.** Painters, photographers,
> filmmakers, illustrators, designers, animators, hobbyists, and people who picked up a pencil
> last week. If you make things, you're one of us. **[Come say hi on Discord](https://discord.gg/artcraft).**

<p align="center">
  <a href="#why-anicraft">Why AniCraft</a> ·
  <a href="#features">Features</a> ·
  <a href="#build-and-run">Build and run</a> ·
  <a href="#headless--agents">Headless &amp; Agents</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#status">Status</a> ·
  <a href="#downloads">Downloads</a> ·
  <a href="#the-crafting-apps">Crafting Apps</a> ·
  <a href="https://discord.gg/artcraft">Discord</a>
</p>

<br>

<table>
  <tr>
    <td width="25%" valign="top">
      <h3>🎛️ Familiar by design</h3>
      Stage, tools, timeline, layers, keyframes, properties and library are where your hands expect them. If you know Adobe Animate or Flash, you already know AniCraft.
    </td>
    <td width="25%" valign="top">
      <h3>⚡ Native and fast</h3>
      A pure Rust engine with a deterministic analytic anti-aliasing CPU oracle renderer, GPU-ready via wgpu / egui, and rayon row parallelism. No Electron, no webview, no waiting.
    </td>
    <td width="25%" valign="top">
      <h3>🎞️ Vector tweens</h3>
      Classic tweens with easing presets and shape tweens with arbitrary contour morphing (<code>geom::morph</code>). The document is plain data, and the picture at a frame is a pure function <code>evaluate(doc, frame)</code>.
    </td>
    <td width="25%" valign="top">
      <h3>🤖 Headless first</h3>
      Every action is an engine command. Drive the exact same engine from the UI, the CLI or the JSON-lines agent channel.
    </td>
  </tr>
</table>

<br>

## Why AniCraft

AniCraft is a clean-room, native 2D vector animation studio designed for animators, motion designers,
and game developers. Built from scratch in pure Rust, it delivers a modern, lightweight, high-performance
alternative to Adobe Animate with zero runtime bloat:

* **Pure Rust & memory safe**: No webviews, no Electron, and no unsafe code (`unsafe_code = "forbid"`).
* **Deterministic CPU oracle renderer**: Analytic anti-aliasing via signed-area coverage accumulation, non-zero and even-odd fill rules, linear and radial gradients, round-join strokes, and multi-threaded band rendering.
* **Non-destructive & live**: Edits render live in the same frame. Layered architecture with structural sharing (`Arc`) keeps undo and history instant and memory-proportional.
* **Interchange ready**: Open Adobe Animate documents (`.fla`, `.xfl`, XFL folders), import SVG drawings straight to the stage, export PNG frames and sequences, animated GIFs with palette quantization, and SVG vector frames.

## Features

### Stage and Drawing Tools
* **Selection & Subselection**: Select, move, nudge with arrow keys, anchor and Bézier handle editing on paths (`Subselection (A)`), add and delete points.
* **Vector Drawing**: Pen (`P`) with smooth handles and corner conversion, Pencil (`Shift+Y`) with Schneider curve fitting and smoothing, Rectangle (with corner radii), Oval, PolyStar, and Line tools.
* **Free Transform (`Q`)**: Interactive scaling, corner/edge handles with proportional Shift and centre Alt modifiers, and rotation with 15° snapping.
* **Fills and Strokes**: Paint Bucket (`K`) with smart gap closure detection, Ink Bottle (`S`), solid colors, linear/radial gradients, and stroke styling.
* **Arrangement & Alignment**: Align left/center/right/top/middle/bottom (with stage alignment support), distribute, rotate 90° CW/CCW, and horizontal/vertical flipping.

### Timeline & Animation
* **Layers & Organization**: Normal layers, guide layers, and mask layers (stencil grouping where masked layers clip cleanly under the mask).
* **Frame Management**: Keyframes (`F6`), blank keyframes (`F7`), frame insertion (`F5`), frame span dragging, multi-frame selection across layers, and cut/copy/paste frames.
* **Classic & Shape Tweens**: Classic tweens with 8 easing presets and custom ease curves; shape tweens morphing between arbitrary vector contours.
* **Onion Skinning**: Visual range before and after the playhead for precise in-betweening.
* **Symbols & Instances**: Graphic symbols with nested timelines, independent loop modes (Loop, Play Once, Single Frame), and first-frame offsets.

### Workflow & Customization
* **Shortcut Editor (`F1`)**: Interactive keymap editor to customize, rebind, or clear any tool or command shortcut.
* **Command Palette (`Ctrl+K`)**: Quick fuzzy search and execution of all engine commands.
* **Safety & Recovery**: Tiered confirmation for destructive actions, unsaved changes guards, and non-overwriting untitled imports.

## Build and run

```sh
cargo build                       # whole workspace
cargo run                         # desktop app with the sample animation
cargo run -- --screenshot shots/now.png   # draw ~120 frames, save the window, exit
cargo test                        # all crates
```

## Headless / agents

AniCraft is headless first: every action in the UI is an engine command, accessible through the CLI or over JSON-lines.

```sh
cargo run -p anicraft-cli -- info --sample
cargo run -p anicraft-cli -- render --sample --frame 12 --out frame12.png
cargo run -p anicraft-cli -- render-seq --sample --out-dir frames --scale 0.5
cargo run -p anicraft-cli -- gif --sample --out animation.gif --fps 24
cargo run -p anicraft-cli -- commands                 # the registry as JSON
cargo run -p anicraft-cli -- exec --sample draw.rect '{"x":10,"y":10,"w":80,"h":40}' --save out.anicraft
cargo run -p anicraft-cli -- serve --sample           # JSON-lines on stdin/stdout
```

The `serve` protocol accepts one JSON request per line:
```json
{"id": 1, "cmd": "timeline.insert_keyframe", "params": {"frame": 10}}
```
And returns structured status:
```json
{"id": 1, "ok": true, "result": {}, "revision": 7, "frame": 0}
```
Send `{"cmd": "quit"}` to exit.

## Architecture

The codebase enforces strict downward-only layering:

```text
crates/geom     L0  Vec2, Affine (decompose/compose), Rect, Path, flattening, hit testing
crates/xml      L0  Bounded XML element reader shared by the importers (XFL, SVG)
crates/doc      L1  Document model, easing, timeline evaluation, sample scene
crates/render   L2  CPU oracle renderer (coverage accumulation, paints, strokes, PNG)
crates/xfl      L2  XFL/FLA import: own XML + zip readers, edges → paths (ADR 0009)
crates/svg      L2  SVG import: shapes, groups, use, styles, transforms, gradients → shapes
crates/engine   L3  Session, undo (with gesture coalescing), command registry, .anicraft format
crates/cli      L4  anicraft-cli: info/render/render-seq/commands/exec/run/new/serve
src/            L5  egui shell (app, menus, stage, timeline, panels, theme)
```

No code below `src/` depends on egui, eframe, or windowing libraries.

### Documents
* [`docs/requirements.md`](docs/requirements.md): Scope, functional requirements, and technology choices
* [`docs/architecture.md`](docs/architecture.md): Philosophy, crate map, document model, renderer, and engine
* [`docs/roadmap.md`](docs/roadmap.md): Milestones M0–M8 and current progress
* [`docs/product-principles.md`](docs/product-principles.md): Design and user experience principles
* [`docs/development.md`](docs/development.md): Development setup, testing, and build hygiene
* [`AGENTS.md`](AGENTS.md): Guide for contributors and AI agents

## Status

> [!IMPORTANT]
> **Status: Early Alpha (Not yet for public consumption).**
> AniCraft is under active early development. The core document model, vector rendering oracle,
> egui shell, classic and shape tweens, and initial XFL/SVG imports are functional, but it is not yet
> ready for production animation work. See [`docs/roadmap.md`](docs/roadmap.md) for milestones M0–M8
> and honest progress.

## Downloads

On GitHub, the [latest release](https://github.com/storytold/anicraft/releases/latest) has every build listed below, and [all releases](https://github.com/storytold/anicraft/releases) has earlier versions and their notes. `<ver>` in the file names is the version number, and `SHA256SUMS.txt` lists a checksum for every file.

### Windows

| Build | Installer | Portable |
|---|---|---|
| x64 (64-bit Intel/AMD) | `anicraft-<ver>-windows-x64.msi` | `anicraft-<ver>-windows-x64-portable.zip` |
| arm64 (Snapdragon and other ARM PCs) | `anicraft-<ver>-windows-arm64.msi` | `anicraft-<ver>-windows-arm64-portable.zip` |
| x86 (32-bit) | `anicraft-<ver>-windows-x86.msi` | `anicraft-<ver>-windows-x86-portable.zip` |

Installers and executables are code-signed.

### macOS

| Build | File | Notes |
|---|---|---|
| App, universal (Apple silicon + Intel) | `anicraft-<ver>-macos-universal.dmg` | Signed and notarized |
| Command-line tool, universal | `anicraft-cli-<ver>-macos-universal.zip` | Signed and notarized |

### Linux

| Format | x86_64 | aarch64 (ARM64) | Notes |
|---|---|---|---|
| AppImage | `anicraft-<ver>-linux-x86_64.AppImage` | `anicraft-<ver>-linux-aarch64.AppImage` | Runs anywhere; updates itself with [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate) (`.zsync` files) |
| Flatpak | `anicraft-<ver>-linux-x86_64.flatpak` | `anicraft-<ver>-linux-aarch64.flatpak` | Sandboxed; `flatpak install --user <file>` |
| Debian/Ubuntu | `anicraft-<ver>-linux-x86_64.deb` | `anicraft-<ver>-linux-aarch64.deb` | |
| Fedora/RHEL/openSUSE | `anicraft-<ver>-linux-x86_64.rpm` | `anicraft-<ver>-linux-aarch64.rpm` | |
| Tarball | `anicraft-<ver>-linux-x86_64.tar.gz` | `anicraft-<ver>-linux-aarch64.tar.gz` | Unpack anywhere |

RISC-V (riscv64): `anicraft-<ver>-linux-riscv64.tar.gz`, a tarball only.

### FreeBSD

| Build | File |
|---|---|
| x86_64 | `anicraft-<ver>-freebsd-x86_64.tar.gz` |

### Web (WebAssembly)

| Build | File | Notes |
|---|---|---|
| Static site | `anicraft-web-<ver>.zip` | Runs in a modern browser; host it on any static server |

## The Crafting Apps

AniCraft is one of the **Crafting Apps**: free, open-source creative tools from the
[ArtCraft](https://getartcraft.com/) team, each written from scratch in Rust and each able to
stand on its own.

| | App | What it's for | Code | Learn more |
|:-:|---|---|---|---|
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32"> | **PhotoCraft** | Image editing: layers, masks, type and real PSD files | [GitHub](https://github.com/storytold/photocraft) | [Website](https://getartcraft.com/apps/photocraft) |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32"> | **VectorCraft** | Vector illustration | [GitHub](https://github.com/storytold/vectorcraft) | [Website](https://getartcraft.com/apps/vectorcraft) |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32"> | **FilmCraft** | Video editing, color and sound | [GitHub](https://github.com/storytold/filmcraft) | [Website](https://getartcraft.com/apps/filmcraft) |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32"> | **LightCraft** | Photo library and raw development | [GitHub](https://github.com/storytold/lightcraft) | [Website](https://getartcraft.com/apps/lightcraft) |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32"> | **PdfCraft** | Reading, organizing and protecting PDFs | [GitHub](https://github.com/storytold/pdfcraft) | [Website](https://getartcraft.com/apps/pdfcraft) |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32"> | **EffectCraft** | Motion graphics and visual effects | [GitHub](https://github.com/storytold/effectcraft) | [Website](https://getartcraft.com/apps/effectcraft) |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32"> | **DesignCraft** | Page layout and publishing | [GitHub](https://github.com/storytold/designcraft) | [Website](https://getartcraft.com/apps/designcraft) |
| <img src="https://raw.githubusercontent.com/storytold/cadcraft/main/assets/app-icon/cadcraft-64.png" alt="" width="32" height="32"> | **CADCraft** | Computer-aided design and drafting | [GitHub](https://github.com/storytold/cadcraft) | [Website](https://getartcraft.com/apps/cadcraft) |
| <img src="docs/brand/artcraft-mark.svg" alt="" width="32" height="32"> | **AniCraft** | **2D vector animation studio · you are here** | [GitHub](https://github.com/storytold/anicraft) | [Website](https://getartcraft.com/apps/anicraft) |

And [**ArtCraft**](https://getartcraft.com/) itself, our AI image and video studio for artists who want real control.

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">Come make things with us</h3>

<p align="center">
  Our Discord is where artists of every kind hang out: people who paint, shoot, draw, cut film,
  set type, and people still figuring out what they like to make. Share what you're working on,
  ask for help, tell us what's broken, or tell us what you wish these tools could do.
  Whatever your medium and however long you've been at it, you're welcome here.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">The Crafting Apps</a> ·
  <a href="https://getartcraft.com/apps/anicraft">AniCraft</a>
</p>

---

## License and credits

AniCraft is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 ArtCraft Team and the AniCraft contributors. Required notices are in [NOTICE](NOTICE).

Bundled fonts, icons, images and other assets keep their own open licenses; each one is listed
with its author, source and license in [ATTRIBUTION.md](ATTRIBUTION.md).

The ArtCraft name, wordmark and logos in [`docs/brand/`](docs/brand/) are trademarks of the
ArtCraft Team and are not covered by this license. They may be used only unmodified, and only as
part of this repository and AniCraft, under [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
Forks and modified versions must remove them.

<sub>Adobe, Animate and Flash are trademarks or registered trademarks of Adobe Inc. in the United States and/or other countries. AniCraft is an independent, open-source project and is not affiliated with, sponsored by or endorsed by Adobe Inc.; these names are used only to describe the workflows and file formats it is compatible with.</sub>

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="docs/brand/artcraft-mark.svg" width="28"></a><br>
  <sub>Made by the <a href="https://getartcraft.com/">ArtCraft</a> team and community.</sub>
</p>
