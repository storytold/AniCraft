# ADR 0006: Platform layer (macOS, Windows, Linux) and stage edit latency

* Status: accepted (2026-10-28)

## 1. Edit latency: the "drag commits after a pause" report

**Finding: not intentional, and not how the target product behaves.** Animate moves artwork live
under the pointer. Two causes, both by-products of ADR 0002's optimizations:

1. **Missing repaint (the real bug).** egui repaints only on demand. A drag edit applied late in
   a frame (after the stage had drawn) was shown only when the *next* event arrived (pointer
   motion, hover timer). When the pointer paused, the last move "committed" after an unrelated
   timeout. Fix: the shell requests a repaint whenever the document revision changes, and the
   stage now renders *after* tool input in the same frame.
2. **Background-only rendering.** Every frame went through the worker thread, so even cheap
   edits showed the previous picture for a frame or two. Fix: an adaptive policy, switchable
   (View › Stage Rendering, and Preferences):

| Mode | Edits | Navigation and playback | When to use |
|---|---|---|---|
| **Auto** (default) | Rendered immediately on the UI thread (row-parallel) while the last frame cost ≤ 12 ms; otherwise in the background, with a vector **outline preview** of the moving selection | Background worker + frame cache + look-ahead | Everyone |
| Immediate | Always immediately | Background | Fast machines, small scenes |
| Background | Always background (with outline preview) | Background | Huge scenes, weak CPUs |

The CPU renderer stays the oracle; only *when* it runs changes.

## 2. Platform layer (`src/platform.rs`)

The policy is one shell with platform decisions isolated in one module, rather than three UIs.

| Concern | macOS | Windows | Linux / BSD |
|---|---|---|---|
| Shortcuts | `Ctrl` in the registry = **⌘**; labels use Apple order and glyphs (⇧⌘S), or "Shift+Cmd+S" when the font lacks them (checked at runtime) | Ctrl labels | Ctrl labels |
| Application menu | **AniCraft** menu: About, Settings… (⌘,), Quit (⌘Q) | File › Exit (Ctrl+Q), Edit › Preferences… (Ctrl+U, as in Animate) | as Windows |
| Window chrome | Unified title bar: full-size content view, transparent title bar (an opaque one covers the menu row: #247), title hidden, menu row inset for the traffic lights | Native frame | Native frame (X11 or Wayland chosen by winit; app id `anicraft` for window rules and docks) |
| File dialogs | Native panels (rfd) | Common dialogs (rfd) | **XDG desktop portal** (rfd). A Preferences switch falls back to a built-in path field on desktops without a portal |
| GPU backend | wgpu → Metal | wgpu → DX12/Vulkan | wgpu → Vulkan; **automatic retry with OpenGL** (glow) if wgpu cannot start (VMs, old GPUs, missing drivers); `--renderer gl` / `ANICRAFT_RENDERER` to force |
| Settings folder | ~/Library/Application Support/AniCraft | %APPDATA%\AniCraft | $XDG_CONFIG_HOME/anicraft |
| Fonts | Bundled (ADR 0005); any installed family selectable in Settings (fontdb scans /System/Library/Fonts, ~/Library/Fonts) | Bundled; Windows\Fonts selectable | Bundled; fontconfig directories selectable |
| HiDPI | Retina scale from winit | Per-monitor DPI | Fractional scaling (Wayland) from winit; Interface scale preference on top |

### Not done yet (needs native APIs or real-machine testing)

* **Native macOS menu bar (NSMenu).** It needs `muda` (MIT/Apache, not in the offline cache) or
  Objective-C calls behind `unsafe`, which policy allows only in an isolated crate (as with
  PhotoCraft's tablet crate). Planned as `crates/macos` (M8). Until then the menu lives in the
  window, following Mac conventions.
* CI on macOS and Linux runners (build, tests, `--screenshot` under Xvfb/Wayland headless).
  **This build was verified on Windows only.**
* Linux performance profiling across Mesa/NVIDIA and X11/Wayland; Intel Mac and Apple Silicon
  numbers for `docs/performance.md`.

## Consequences

* New UI code never branches on the OS itself; it asks `platform::`.
* Shortcut labels in menus, the palette and the F1 sheet go through `platform::shortcut_label`.
