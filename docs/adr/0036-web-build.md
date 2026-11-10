# ADR 0036: Web build of the editor (wasm32)

* Status: accepted (2026-11-09)
* Relates to: ADR 0006 (platforms), ADR 0015 (crash-recovery autosave), ADR 0017 (WebAssembly
  plug-ins), ADR 0019 / 0032 (GPU stage), ADR 0020 (audio), ADR 0004 (destructive actions)

## Context

The requirements say the engine layers build for `wasm32-unknown-unknown`, and M8 lists a web
build of the editor. egui and eframe run in a browser (`eframe::WebRunner` on a canvas). The
browser differs from a desktop in the ways that matter to us:

* There is **no file system**: files come in through a file picker and leave as downloads.
  Every engine command that reads or writes a file takes a path.
* There are **no threads** on `wasm32-unknown-unknown`: `std::thread::spawn` panics,
  `Builder::spawn` returns an error, `thread::sleep` panics.
* `std::time::Instant::now()` and `SystemTime::now()` **panic**.
* No process can be started (ffmpeg), no folder holds plug-ins or recovery copies, a page can't
  see the computer's fonts, and wgpu's objects belong to one thread and start asynchronously.

A build that compiles can still trap on the first clock call, so "it builds" is not enough.

## Decision

1. **One program, two targets.** The `anicraft` binary is the web editor when built for
   wasm32: `main` calls `web::start`, which runs the editor on the page's canvas. There is no
   second app crate. Desktop-only code stays compiled but unused there (the crate allows dead
   code on wasm32 only). OS-specific decisions stay in `platform.rs` (ADR 0006); `src/web.rs`
   is the browser glue it calls: start-up, file picker, downloads, storage, Web Locks, and the
   leave-page guard.

2. **Files: the engine reads and writes through `engine::vfs`.** On the desktop every `vfs`
   function is the `std::fs` call it names, so nothing changes there. In the browser it is an
   in-memory store (bounded at 1 GiB, paths normalized, `..` refused). The shell puts a picked
   file into it and runs the same command with that path (`file.open {"path":
   "walk.anicraft"}`), and after any `file.*` command hands what was written to the browser as
   downloads (`vfs::take_written`, which also empties the store). So every command keeps its
   one implementation for the editor, the CLI, MCP and the web, and the downloaded name is the
   document's name. Files are written on the command's thread, never from the rayon pool (the
   PNG sequence now renders in parallel batches and writes after each). The store is per
   thread so native tests can switch one thread to it (`vfs::use_memory`); the commands'
   browser behaviour is tested natively that way.
   * `.fla` archives open from bytes (`anicraft_xfl::import_fla`). XFL folders can't come from
     a browser and say so.
   * `file.open` gains `unsaved?` (open as unsaved work) and `untitled?` (with no file name),
     which the browser's Restore uses. Older callers are unaffected.
   * Video export (ffmpeg writes the file itself) says it can't run in a browser.

3. **Save is a download.** Save and Save As ask for the file name with the in-app box (the
   browser decides the folder) and download. A saved or opened document keeps its name, so
   the next Save downloads it again without asking. Opening, importing and installing use the
   browser's file picker, filtered as the desktop dialogs are.

4. **Autosave goes to the browser's local storage** (ADR 0015 in a browser). `web::BrowserSaver`
   has the desktop `Autosaver`'s calls (write, clear, poll, finish), so `autosave.rs` keeps one
   flow; it writes `anicraft.recovery.<session>.doc` and `.meta` synchronously (there is no
   thread; serializing is milliseconds for ordinary documents). A tab that is open holds a
   **Web Lock** named after its session for its whole life (the request's promise never
   settles), so at start the copies offered are exactly those of tabs that closed or crashed.
   This is the desktop's lock-file rule (the OS drops the lock when the process dies). Without
   Web Locks (old browsers, insecure pages) every other session counts as closed; restoring is
   a copy, so the worst case is a second copy of open work. Restore opens the copy through
   `file.open {unsaved, untitled}` under `recovered/<session>/<name>`. Discard deletes it.
   Preferences live in local storage too. A full storage reports "Autosave failed" in the
   status bar, as a full disk does.

5. **Leaving the page with unsaved work asks first** (`beforeunload`), the browser's version
   of the close guard.

6. **Threads: rayon's own fallback, not a feature.** rayon-core runs its pool on the calling
   thread when threads are unsupported (it was built for exactly this), so every `par_iter`
   works unchanged and comes back parallel if the web ever gets threads. Only the
   fire-and-forget drops (`rayon::spawn`) would never run there, so they go through
   `anicraft_render::drop_in_background`, which drops inline on wasm32. The render service
   already renders on the caller when no worker starts. Other threads (font scan, sound
   decode) use `Builder::spawn` and simply don't start.

7. **Clocks: `web-time`** (MIT OR Apache-2.0, already in the tree through winit). It is
   `std::time` on the desktop and `performance.now()` in a browser. Plug-in runs keep their
   fuel limit there; the wall-clock limit is native-only, as it already was.

8. **The stage draws on the CPU, and sound is silent, in the first slice.** The `gpu-stage`
   and `sound-output` features become the `gpu_stage` and `sound_output` cfgs (`build.rs`),
   which are off on wasm32. So `cargo build --target wasm32-unknown-unknown` works with the
   default features, and the web editor needs no special flags. `anicraft-gpu` itself builds
   for wasm32 (its layers plan one after another there, since wgpu's handles aren't `Sync`),
   so a later slice can drive it from eframe's device. The window draws with WebGL2 (glow):
   every current browser has it, and leaving wgpu out of the web shell keeps the download
   smaller.

9. **Fonts.** A page can't see the computer's fonts, so text uses the faces the app embeds
   (Inter and JetBrains Mono when built with craft-fonts), or egui's own Ubuntu face when
   there are none, so the Text tool always has a face to draw with.

10. **Build: `wasm-bindgen` CLI, not trunk.** `scripts/build-web.sh` and `.ps1` run cargo for
    the `web` profile, then `wasm-bindgen --target web`, then copy `packaging/web/index.html`
    and the icon into `web/` (git-ignored; nothing generated is committed). One small tool,
    MIT OR Apache-2.0, pinned by the script to the version `Cargo.lock` pins for the
    `wasm-bindgen` crate. Glue from another version doesn't work, and the script says which
    version to install. Trunk (also MIT OR Apache-2.0) bundles a dev server and downloads
    wasm-bindgen and wasm-opt binaries itself; we need neither, and a build that fetches
    binaries is harder to reproduce. The `web` profile is the release build at `opt-level =
    "s"`, stripped, with the renderer and geometry crates at `opt-level = 3`, because they
    draw the stage on one thread there.

11. **Commands the browser can't run stay where artists expect them, dimmed, with the reason
    on hover** (`platform::unavailable`): Export Video, and installing, removing or reloading
    plug-ins (they live in a folder). Exit and Help › Project Files are hidden; a tab is
    closed with the browser.

12. **CI** (`.github/workflows/web.yml`) builds every crate for wasm32, runs
    `crates/engine/tests/wasm.rs` inside WebAssembly in Node with wasm-bindgen's test runner
    (render without a thread, save/open/exports through memory, a plug-in run), and builds the
    bundle with the script.

## Measurements (2026-11-09, Windows 10, Rust 1.97)

| | raw | gzip -9 |
|---|---|---|
| `web` profile (as chosen) | 8973 KB | 3278 KB |
| `opt-level = "s"`, stripped, everything | 9119 KB | 3317 KB |
| `opt-level = "z"`, stripped | 9375 KB | 3202 KB |
| `opt-level = 3`, stripped | 11 324 KB | 4032 KB |

`scripts/build-web.sh` takes 41 s once dependencies are built. The engine's WebAssembly tests
run in 0.6 s in Node. The whole editor module instantiates in Node and runs `main` to its "no
canvas" exit without a panic.

## What only a browser can confirm

That the canvas starts and draws (WebGL2), that the file picker opens from a menu click in
every browser (they require a recent user gesture; Safari is the strictest), that downloads
and multiple downloads (PNG sequence) arrive, local storage quota behaviour, Web Locks across
tabs, and the `beforeunload` prompt. None of these could be run here; the CI job and the Node
tests cover everything below them.

## Consequences

* Engine file I/O has one more rule: go through `vfs`, on the command's thread.
* Browsers keep about 5 MB of local storage per site, so autosave of documents with large
  images or sounds can fail in the browser, and the status bar says so. IndexedDB (async,
  much larger) is the next step.
* Not yet: the GPU stage on WebGPU, sound through Web Audio (cpal's web backend), the PNG
  sequence as one zip download, plug-in installs into browser storage, a web-hosted build,
  wasm-opt, threads (wasm-bindgen-rayon with cross-origin isolation), and opening XFL folders.
