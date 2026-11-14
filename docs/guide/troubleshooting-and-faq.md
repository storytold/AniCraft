# Troubleshooting and FAQ

When something doesn't work the way you expect, start here. The first half covers practical
problems and their fixes; the second half answers the questions people ask most.

On macOS, press ⌘ where this guide says Ctrl, and ⌥ where it says Alt.

## The window won't open, or stays blank

AniCraft draws its window with your graphics card through a modern graphics interface (Vulkan,
Metal or DirectX 12). If that fails to start, which happens on some older cards, laptops with two
graphics chips, virtual machines and Linux setups, AniCraft tries the next way by itself: on
Windows, DirectX 12 on the built-in graphics chip, and then OpenGL everywhere. If a graphics
driver crashes or hangs while AniCraft starts, the next start tries that way last.

If no way works, AniCraft says so in a message and names its **start-up log**, a text file that
records each way it tried and why it failed. Please attach it to your bug report. It is
`anicraft.log` in the `Logs` folder of the [settings folder](#where-settings-live), with the
previous run's beside it as `anicraft.previous.log`.

If the window opens but flickers, goes black for a moment or looks garbled, choose another way
in **Preferences › Stage › Window graphics** (for example **DirectX 12** or **OpenGL** on
Windows) and restart AniCraft. From a terminal, the same choice is:

```sh
anicraft --renderer gl        # or dx12, vulkan, metal
```

To make it stick without Preferences, set the environment variable `ANICRAFT_RENDERER` for your
account. `--renderer wgpu` (or `ANICRAFT_RENDERER=wgpu`) goes back to the automatic choice.

## The stage is slow, flickers or looks wrong

The stage is drawn on the graphics card by default, and on the CPU whenever there is no card
that can, or a frame needs something the card can't draw. The picture is meant to be identical
either way.

1. Open **Preferences › Stage** (**Ctrl+U**). The note under **Renderer** says which card is
   drawing, or why the CPU took over.
2. Choose **CPU (reference)** to rule the graphics card out. If the problem goes away, please
   report it with the name of your card (see [Reporting a bug](#reporting-a-bug)).
3. If dragging feels heavy on a big scene, try **Rendering › Background**, which never makes the
   window wait. **Auto** is the best balance for most work.

The same choices are in **View › Stage Rendering**. If the note says `ANICRAFT_STAGE_RENDERER`
is set, that environment variable overrides Preferences; remove it to choose again.

Exports always use the CPU renderer, so a stage problem never ends up in your files.

## The graphics card stopped in the middle of my work

Under heavy load (a game or a render running beside AniCraft, a very large scene) a graphics
driver can stop answering or reset itself. AniCraft keeps going:

* **The stage** switches to the CPU at once and the status bar says so. The picture is the same,
  only slower on big scenes. AniCraft tries the graphics card again after a few seconds, then
  after longer pauses (and sooner when you resize the window); the status bar says when it is
  back. After several failures in a row the CPU keeps the stage for the rest of the session.
* **A driver error** the window survives is written to the log, your unsaved work is autosaved
  straight away, and the status bar says AniCraft carried on.
* **If the window itself loses the graphics card**, AniCraft saves your work, closes the window
  and opens it again a safer way (on Windows, DirectX 12 on the built-in graphics chip, then
  OpenGL). The **Recover unsaved work** window offers your work back. The next three starts try
  that safer way first too, then the fastest way gets its chance again.

If it keeps happening, update the graphics driver, or choose another way in **Preferences ›
Stage › Window graphics**. The log (`anicraft.log`, in the `Logs` folder of the
[settings folder](#where-settings-live)) says what the driver reported; please attach it to a
bug report.

## Fonts look different or are missing

* **The interface font.** AniCraft's builds normally include the Inter and JetBrains Mono
  typefaces. A build made without them uses simpler built-in fonts instead. Either way you can
  pick any installed font in **Preferences › Appearance › Interface font** and **Data font**.
* **Text in your animation.** The Text tool offers the fonts installed on your computer, plus a
  built-in face. Text is stored in the document as outlines, so it looks the same on a computer
  that doesn't have the font; to change the wording or style there, install the font.
* **Animate files.** When a document uses a font you don't have, AniCraft uses a close
  substitute and names the missing family in the import report. Install the font and open the
  file again. See [Opening Animate files](opening-animate-files.md).
* **In the web version**, a page can't see your computer's fonts, so text uses the built-in
  faces.

## No sound on Linux

AniCraft plays sound through ALSA, which PipeWire and PulseAudio reach through their ALSA
plug-ins. If playback is silent or the status bar says "no sound output device":

1. Make sure the ALSA library is installed (`libasound2`, called `alsa-lib` on Fedora and Arch).
   The deb and rpm packages ask for it.
2. On PipeWire, install `pipewire-alsa`; on PulseAudio, its ALSA plug-in (often in
   `pulseaudio` or `alsa-plugins-pulseaudio`).
3. Check that another program can play sound, then restart AniCraft.

Sounds still import, show on the timeline and go into Export Audio and Export Video even when
playback is silent. A message that "this build of AniCraft has no sound output" means it was
built without sound playback; use a release build.

## Video export doesn't work

**File › Export › Export Video…** hands the frames to ffmpeg, which you install yourself (AniCraft
doesn't bundle a video encoder). Install ffmpeg so it is on your `PATH`, or set the environment
variable `ANICRAFT_FFMPEG` to the full path of the `ffmpeg` program. The web version can't run
ffmpeg at all: export an animated GIF, animated PNG or WebP, a PNG sequence or an HTML page
instead. See [Exporting and publishing](exporting-and-publish.md).

## AniCraft closed unexpectedly

Your unsaved work is probably safe. AniCraft keeps a recovery copy in the background (every
minute or every 25 changes by default, see **Preferences › Files › Autosave**).

1. Start AniCraft again. The **Recover unsaved work** window lists what was left behind.
2. **Restore** opens a copy as unsaved work. Save puts it back where it was (if the file was saved
   after the copy, AniCraft asks for a new name instead, so nothing is overwritten).
3. **Discard…** deletes a copy for good, after a second click.
4. **Later** keeps the copies. **File › Recover Autosaved Documents…** brings the window back.

Recovery copies never replace your own files, and they go away when you save or close normally.
If the status bar ever says "Autosave failed", save your work by hand; AniCraft keeps trying.

## Where settings live

Preferences, shortcuts, recent files, recovery copies, your templates and your plug-ins all live
in one settings folder. Its path is shown at the bottom of the Preferences window.

| System | Settings folder |
|---|---|
| Windows | `%APPDATA%\AniCraft` |
| macOS | `~/Library/Application Support/AniCraft` |
| Linux and FreeBSD | `~/.config/anicraft` (or `$XDG_CONFIG_HOME/anicraft`) |
| Web version | the browser's storage for the site |

Inside it: `prefs.json` (preferences and shortcuts), `Recovery` (autosaved copies), `Templates`
(your templates), `plugins` (installed plug-ins), `Thumbnails` (start screen pictures) and `Logs`
(the start-up log).

To start over with default preferences, quit AniCraft and rename or delete `prefs.json`. A
damaged preferences file never stops AniCraft from starting: it falls back to the defaults.

## Opening files from the desktop

* **Windows and Linux**: double-click a `.anicraft` file. `.fla` and `.xfl` files are offered under
  **Open with**. You can also drop any of them on the AniCraft window.
* **macOS**: double-click a `.anicraft`, `.fla` or `.xfl` in Finder, drop it on AniCraft's Dock
  icon, or use **Open With › AniCraft**, whether AniCraft is running or not. If you have unsaved
  changes, AniCraft asks first. This is new and still being checked on real Macs; if it doesn't
  work for you, use **File › Open…** and please report it.
* **An old `.fla`**: files saved by Flash CS4 or earlier use an older format. AniCraft says so;
  open the file in Animate, save it again, then open that copy.
* **A damaged `.fla`**: AniCraft refuses it with a sentence that says what is wrong, and never
  writes anything from it to your disk.

## Other desktop quirks

* **Linux: the open and save dialogs don't appear.** Your desktop may have no XDG desktop portal.
  Turn off **Preferences › Files › Use the system's open and save dialogs**, and AniCraft asks for
  a path instead.
* **Wayland: Test Movie opens in a different place each time.** On Wayland the desktop decides
  where windows go. To keep the movie inside the editor, turn off **Preferences › Test Movie ›
  Play Test Movie in a window of its own**.
* **A shortcut doesn't do anything.** Check what the key is bound to in **Help › Keyboard
  Shortcuts** (**F1**): type the key in the filter. See
  [Preferences, shortcuts, accessibility and languages](preferences-shortcuts-accessibility.md).

## The web version

The web version is the same program running in a browser, with some limits:

| Not in the web version yet | What to do |
|---|---|
| Sound playback while editing | Sounds still import and export (WAV, HTML page); play them in the desktop app. |
| Video export | Export a GIF, animated PNG or WebP, a PNG sequence or an HTML page. |
| Installing plug-ins and saving your own templates | Use the desktop app (the built-in templates work). |
| Fonts on your computer | Text uses the built-in faces. |
| XFL folders | Open the `.fla` instead. |
| Large autosaves | The browser keeps only a few MB per site: save often. |
| The stage on the graphics card, and parallel rendering | Large scenes and exports are slower. |

Saving and exporting download the file, usually to your Downloads folder. A PNG sequence
downloads frame by frame, and your browser may ask once to allow several downloads.

## Reporting a bug

Please report problems on [GitHub issues](https://github.com/storytold/AniCraft/issues). A good
report has:

* the version (**Help › About AniCraft**, or `anicraft-cli --version`) and your system;
* what you did, step by step, what you expected and what happened instead;
* a screenshot, and the document if you can share it (for Animate files, the import report too).

Search the existing issues first: someone may have found a workaround. On the desktop, **Help ›
Check for Updates…** tells you whether a newer version is out.

## FAQ

### Is AniCraft free?

Yes. AniCraft is free and open source, for any use, including commercial work. Downloads are on
the [releases page](https://github.com/storytold/AniCraft/releases).

### What is the licence?

AniCraft is dual-licensed under the MIT licence or the Apache License 2.0, at your choice. The
licences cover AniCraft itself; the animations you make with it are yours. **Help › Licenses &
Notices** shows the full texts, including those of the fonts and icons AniCraft ships.

### Is AniCraft made by Adobe, or affiliated with Adobe?

No. AniCraft is an independent, open-source project. It is not affiliated with, sponsored by or
endorsed by Adobe. It works the way people who know Adobe Animate expect, and it opens Animate
documents, but none of Adobe's code or assets are in it.

### Can it open my `.fla` files?

Yes: `.fla`, `.xfl` and XFL folders, with layers, keyframes, classic and shape tweens, symbols,
bitmaps, text, sounds, masks, filters and more. Anything that doesn't come across yet is listed in
a report after opening. Motion tweens from Animate files are not imported yet. See
[Opening Animate files](opening-animate-files.md).

### Can it save back to `.fla`?

No. AniCraft saves its own `.anicraft` format, and an opened Animate file comes in as a new,
untitled document, so saving never overwrites the original.

### Can it export SWF?

No. AniCraft doesn't export SWF, and browsers no longer play it anyway. For the web, use **File › Export › Export HTML Canvas…**, which plays your movie, buttons and
scripts in any browser. AniCraft also exports video (with ffmpeg), animated GIF, animated PNG and
WebP, Lottie, sprite sheets, SVG and PNG frames and sequences, and audio as WAV.

### Does ActionScript work?

Partly. Frame scripts in ActionScript 3.0 that control the timeline (stop, play, go to a frame or
label), respond to mouse and keyboard events, and use simple logic run in Test Movie and the HTML
canvas export. Scripts that use more than that are kept with the document but don't run, and the
Actions panel says why. See [Interactivity](interactivity.md).

### Which systems does it run on?

Windows, macOS, Linux and FreeBSD, and a web version that runs in a browser. It has been tested
most on Windows so far.

### Is it ready for production work?

Not yet. AniCraft is an early alpha: much works, but expect rough edges, and save often. The
honest, detailed status is in the project's roadmap on GitHub.

### Can I open several documents at once?

Not yet. AniCraft works on one document at a time for now. Opening another asks you to save
the current one first if it has unsaved changes.

### Is the interface available in my language?

Not yet. The interface is ready for translation, but it is in English only today. See
[Interface languages](preferences-shortcuts-accessibility.md).

### Can an AI assistant use AniCraft?

Yes, if you want it to. `anicraft-cli mcp` lets an assistant that supports the Model Context
Protocol draw, animate and export through AniCraft's commands. It is never on unless you set it
up. See [Working headless](working-headless.md).
