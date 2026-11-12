# ADR 0062: Test Movie in its own window, and opening documents on macOS

* Status: accepted (2026-11-11)
* Relates to: ADR 0006 (platform layer), ADR 0022 (accessibility), ADR 0024 (Test Movie),
  ADR 0026 (packaging), ADR 0036 (web build), ADR 0040 (interface languages), ADR 0052
  (Test Movie keyboard), ADR 0059 (files from the desktop)

## Context

Test Movie (ADR 0024) floated inside the editor until AniCraft could open more than one window.
In Animate, Control › Test Movie opens the movie in a window of its own, at the stage's size,
which artists put on a second screen or beside the editor and close with the usual
window-close key. eframe 0.36 can open more windows (viewports) on Windows, macOS, X11 and
Wayland, but not in a browser.

The packaging work (ADR 0026) declared AniCraft's document types to macOS, but double-clicking
a `.anicraft` file in Finder started AniCraft without opening the file. macOS does not put the
file on the command line. It sends an Apple Event (`kAEOpenDocuments`), both at launch and
while the app runs, and winit 0.30 has no event for it (`WindowEvent::DroppedFile` is a drop on
a window, which is something else). winit's docs say to install your own `NSApplication`
delegate, which needs `unsafe` Objective-C. AniCraft forbids `unsafe` in its code.

Windows and Linux already pass the document on the command line (`"%1"` in the MSI's file
association, `Exec=anicraft %f` in the desktop entry). Nothing had tested that.

## Decision

### Test Movie

1. **An immediate viewport**, not a deferred one. Test Movie plays with the editor's own
   state: the sound output (ADR 0039), the plug-in registry the frame scripts run on, the
   shortcut table, and the preferences. An immediate viewport's callback runs inside the
   editor's frame with `&mut App`, so nothing is shared across threads and there is no
   `Arc<Mutex<…>>` around the sound device. A deferred viewport would need `Send + Sync`
   state, and a second copy of the shortcut dispatch. The cost is that the editor redraws
   whenever the movie does. That redraw is cheap, because the stage keeps its tiles cached
   (ADR 0007) and the movie only renders when its state changes. If profiling shows a
   problem, the movie can move to a deferred viewport behind the same `show`.
2. **One body, two hosts** (`test_movie::body`). The controls, the movie under the pointer
   and keyboard, and the Output are drawn once. They go either in the window of its own
   (`own_window`: an `egui::CentralPanel` in the viewport) or in the floating egui window
   (`inside`). Behaviour can't differ between the two.
3. **Where it plays** is `platform::movie_host` (ADR 0006: OS decisions live in
   `platform.rs`):
   * the web build: inside, and it says *"a web page can't open a window of its own"*;
   * `Context::embed_viewports()` true (eframe sets that wherever it has no multi-window
     support; headless contexts too): inside, and it says the system can't open another
     window;
   * **Preferences › Test Movie › Play Test Movie in a window of its own** off: inside, with no
     note (the user chose it). This is also the way out on a Wayland setup whose compositor
     handles a second window badly. We can't detect that from inside the app, so we don't
     guess.
   * otherwise its own window.
   We decide before calling `show_viewport_immediate`. We never rely on egui's own embedded
   stand-in, which panics if its window is hidden (golden rule 1).
4. **Size, title, place.** The window opens at the stage's size at 100 % plus the controls,
   as in Animate. If that doesn't fit 85 % of the screen it is made smaller with the aspect
   kept, and never smaller than 480 × 320 (`window_size`). The size is worked out once the
   screen's size is known, and kept across a restart, so Ctrl+Enter doesn't move or resize
   the window. The title is *"Test Movie — <document>"*, in the interface language. The
   window has the app id and icon (`platform::secondary_window`), so docks and taskbars group
   it with the editor.
   **Its place is remembered** in the preferences (`test_movie_pos`: the outer top-left
   corner, in points). It is saved when the window closes and when the app quits, and not in
   screenshot runs. A remembered place is only used if it is on or beside the editor's screen
   (`usable_position`), so a monitor that has since been unplugged can't strand the window off
   screen. Wayland never tells apps where their windows are, and doesn't let them choose:
   there the desktop places the window, Preferences says so (`window_positions`), and nothing
   is saved.
5. **Keyboard** (ADR 0052). The window comes to the front with the keyboard when it opens or
   restarts (`ViewportCommand::Focus`, then focus on the movie surface; egui keeps focus per
   viewport). The movie's keys go to its listeners as before. In the window of its own,
   **Esc** and **Ctrl+W (⌘W)** close it, **Ctrl+Q (⌘Q)** quits through the editor's close
   guard, and the editor's other shortcuts with Ctrl (⌘) or Alt still work there (Restart is
   Ctrl+Enter). They are collected in the viewport and run after it, in the editor's context,
   so a command that talks to "the window" means the editor's. The floating window keeps Esc
   from anywhere and gains Ctrl+W while the movie has the keyboard. The OS close button closes
   it like Esc.
6. **Screenshots.** eframe 0.36 doesn't capture an immediate viewport
   (`ViewportCommand::Screenshot` reaches only windows it paints by themselves), so
   `--screenshot` captures the editor only. `--open testmovieinside` shows the floating
   fallback in the editor's screenshot. The window of its own was checked from a capture of
   that window alone (PrintWindow on Windows).

### Documents macOS hands over

7. **`fmv-macos-events` 0.1.0** (MIT OR Apache-2.0, crates.io, pinned with `=`, about 170
   lines, read in full) registers AppKit's open-documents (`aevt/odoc`) and quit (`aevt/quit`)
   Apple Event handlers. It does this before the event loop starts and again on
   `NSApplicationWillFinishLaunching`, so the document that launched the app isn't missed. It
   hands back plain `PathBuf`s through an inbox on the main thread. It does **not** replace
   winit's application delegate. Its `unsafe` stays inside it, and our code stays
   `unsafe`-free. Its objc2 / objc2-app-kit / objc2-foundation are the versions wgpu already
   brings, so no other crate is added. Other options:
   * our own delegate or Apple Event handler: `unsafe` in our code, which is forbidden;
   * `bevy_macos_open_files`: tied to Bevy;
   * waiting for winit: no released winit has the event yet.
   It is a young crate from one author. It is pinned exactly, `Cargo.lock` holds its
   checksum, and any new version is read again before it comes in. It is replaced as soon as
   winit or eframe deliver the event.
8. **One path for every platform.** `platform::listen_for_documents` (main, before
   `run_native`, kept until it returns) and `platform::wake_on_requests` (the editor repaints
   when one arrives) are the only macOS code. Requests (`SystemRequest::Open(paths)` /
   `Quit`) go through a bounded queue (64) that `App::frame` drains first thing.
   * **Open** goes through the same routing as a drop on the window (ADR 0059): the first
     `.anicraft` / `.fla` / `.xfl` opens via `file.open` through `request`, so **unsaved
     changes ask first**. Other files in the same request are left out with a reason, and a
     request with no document says what AniCraft opens. The start screen gives way.
   * **Quit** (the Dock's Quit, logging out) closes the editor's window, which the close
     guard intercepts when there is unsaved work. The handler replaces AppKit's default, which
     would terminate without asking.
9. **Command-line documents** (Windows, Linux) are now parsed by `main::parse_args`, a pure
   function. It ignores `-psn_…`, which older macOS added when Finder started an app. A test
   checks that a path with spaces arrives as the file and that `open_session` opens it instead
   of the sample.

## Consequences

* Test Movie can sit on another screen while the editor stays usable. Closing it is the usual
  key. The web build and anyone who prefers it keep the floating window.
* The editor redraws while the movie plays (point 1). This is measured and recorded in the PR.
* One small third-party crate holds `unsafe` macOS code on our behalf. It is a supply-chain
  risk, mitigated by an exact pin and a full read, and listed for the lead.
* `fmv-macos-events` declares `rust-version = 1.95`, so the workspace's declared minimum moved
  from 1.90 to **1.95** (`Cargo.toml`, `docs/requirements.md`) on every platform, one MSRV for
  all. CI and releases already build with the current stable.

## Only a Mac can verify

The crate compiles and passes clippy for `aarch64-apple-darwin` from Windows, but nobody has run
it on a Mac yet. Check on a Mac, from a signed DMG install:

1. Double-clicking a `.anicraft` while AniCraft is closed starts it with that document (not the
   start screen).
2. Double-clicking another document while it runs opens it, and asks first if there are
   unsaved changes.
3. Dropping a `.fla` on the Dock icon, and Finder › Open With › AniCraft, open it.
4. Dock › Quit with unsaved work asks to save. Logging out does the same.
5. The Test Movie window opens at the stage's size, comes to the front, ⌘W and Esc close it,
   ⌘↩ restarts it, and it reopens where it was.

## Not in this slice

* Several documents at once: AniCraft has one document window, so a second document replaces
  the first (after asking). Windows and Linux start a second AniCraft for a second
  double-click; a single-instance hand-over is later.
* `file://` URLs from Linux desktops that pass `%u`. Our desktop entry passes `%f` paths.
* Test Movie as a deferred viewport (point 1), a full-screen mode, zoom inside the movie
  window, and remembering its size.
* Detecting a Wayland compositor that fails to show a second window: the preference is the
  answer for now.
