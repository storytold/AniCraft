# Plug-ins

Plug-ins add new commands to AniCraft: generators that draw for you, effects that change the
selection, and scripts that steer a movie while it plays. This chapter shows how to install, run
and manage them, and what a plug-in is and isn't allowed to do.

On macOS, press ⌘ where this guide says Ctrl, and ⌥ where it says Alt.

## What a plug-in is

A plug-in is a single `.wasm` file (a WebAssembly module). The same file works on Windows, macOS
and Linux. There are two kinds:

* **Command plug-ins** appear in **Commands › Plug-ins**. A generator, such as a star field
  scattered over the stage, works on its own. A selection effect, such as one that jitters the
  selected shapes, needs something selected first.
* **Frame-script plug-ins** don't appear in the Commands menu. You pick one for a keyframe in the
  Actions panel, and it runs while the movie plays in Test Movie. See
  [Interactivity](interactivity.md) for keyframe actions.

A plug-in changes your document only through AniCraft's own commands, the same ones the menus
use. So everything it does is checked like any other edit, and the whole run is a single step
you can undo.

Plug-ins belong to AniCraft on your computer, not to a document. They stay installed when you
open or create documents, and a document never contains a plug-in.

## Installing a plug-in

1. Choose **Commands › Install Plug-in…**.
2. Pick the `.wasm` file.
3. AniCraft checks the file and copies it into its plug-ins folder. The status bar says
   "Installed Star Field 1.0.0 — find it in Commands › Plug-ins" (with the plug-in's own name and
   version).

If a plug-in with the same id is already installed, installing replaces it, and AniCraft asks
first because the old file is overwritten. The status bar then says "Updated".

You can also copy `.wasm` files into the plug-ins folder yourself and choose **Commands › Reload
Plug-ins**. Every `.wasm` in the folder loads when AniCraft starts. The folder is called
`plugins` and sits in AniCraft's settings folder:

| System | Plug-ins folder |
|---|---|
| Windows | `%APPDATA%\AniCraft\plugins` |
| macOS | `~/Library/Application Support/AniCraft/plugins` |
| Linux | `~/.config/anicraft/plugins` (or `$XDG_CONFIG_HOME/anicraft/plugins`) |

If a file can't be loaded, AniCraft starts anyway, skips it and names it in the status bar.

Only install plug-ins from people you trust, as you would with any download. The sandbox below
keeps them away from your files, but a plug-in can still change the open document in ways you
didn't expect. Undo puts it back.

## Running a plug-in

1. If the plug-in works on a selection, select some artwork first. Until you do, its menu item
   is greyed out, and the tooltip says "Select some artwork first".
2. Choose it in **Commands › Plug-ins**, or press **Ctrl+K** and type its name: the command
   palette finds plug-ins too.
3. A plug-in without settings runs straight away. One with settings (its name ends in "…") opens
   a dialog first.

### The settings dialog

The dialog is built from the plug-in's own settings: sliders for numbers, checkboxes, menus for
choices and colour swatches.

* **Preview on the stage** is on, so every change you make shows on the stage at once.
* **OK** (or **Enter**) keeps the result.
* **Cancel** (or **Esc**) leaves the document exactly as it was.

Whatever you tried in the preview, OK records one undo step named after the plug-in. The status
bar says how to undo it, for example "Star Field — Ctrl+Z to undo".

Most plug-ins give the same result every time for the same settings. Plug-ins that scatter or
jitter usually have a **seed** setting: change it for a different arrangement.

## Managing plug-ins

Choose **Commands › Manage Plug-ins…** to open the Plug-ins window. It lists every installed
plug-in with its name, version, author, id and size, and its description.

* **Run** (or **Run…** for plug-ins with settings) runs it, as the menu does.
* **Remove…** uninstalls it. Because this deletes the plug-in's file from the plug-ins folder,
  AniCraft asks first, and it can't be undone. Keep a copy of the `.wasm` if you might want it
  again.
* **Folder** shows where the plug-ins folder is. **Show Folder** opens it on your computer.
* **Install Plug-in…** and **Reload** do the same as the Commands menu items.

When nothing is installed yet, the window says so: "No plug-ins installed yet".

## The sandbox: what plug-ins may do

Plug-ins run sandboxed. In the words of the Plug-ins window: they can change the open document
through AniCraft's commands, never your files.

A plug-in **can**:

* read the stage size, frame rate, scenes, layers and the current selection;
* read the elements on the active layer at the playhead;
* run AniCraft's editing commands: draw shapes, add layers, move, transform and recolour artwork,
  add keyframes and so on, with the same checks as the menus;
* write lines to its own log, which AniCraft shows when the run fails.

A plug-in **cannot**:

* open, save, import or export files, or touch anything else on your disk;
* reach the network, the clock or other programs;
* install, run or remove plug-ins;
* undo or redo (the whole run is one undo step);
* cut, copy or paste: your clipboard stays yours;
* run anything that would need a "this cannot be undone" confirmation.

Each run starts fresh, so nothing a plug-in did last time carries over. A run also has limits on
how long it may take (5 seconds), how much memory it may use and how much it may add to the
document. If a plug-in goes over a limit, fails or crashes, the run stops, the document goes back
to how it was before the run, and the status bar says why.

While a command plug-in runs, AniCraft waits for it, at most for its time limit. Frame-script
plug-ins have much tighter limits per frame, and if one runs out the movie keeps playing.

## Frame-script plug-ins

A frame-script plug-in reads where the movie is (frame, scene, pointer, which movie clips are
playing) and sends timeline actions back: stop, play, go to a frame or label, next or previous
frame or scene, trace a message, or set an instance's position, alpha, visibility, rotation or
scale. It never changes the document, so the stage and every export stay as you made them.

To use one:

1. Install it as above.
2. Select a keyframe and open the Actions panel (**Window › Actions**, **F9**).
3. Set **Type** to **Plug-in**, choose the plug-in, and pick its **Entry** point if it has
   several.
4. Test the movie with **Control › Test Movie** (**Ctrl+Enter**). Messages from the script appear
   in the Output panel.

The keyframe remembers which plug-in it runs, but the plug-in itself stays on your computer. If
you send the document to someone who doesn't have it installed, the Actions panel says so, and
Test Movie notes it and plays on without it.

## Plug-ins in other places

* **The web version** can't install, remove or reload plug-ins, because they live in a folder on
  your computer. Use the desktop app.
* **On the command line**, plug-ins load only when you ask, so scripted runs are repeatable. See
  [Working headless](working-headless.md).

## What isn't there yet

* Plug-ins that add export formats, easing presets, panels or other parts of the interface.
* Running a long plug-in in the background with a progress bar and Cancel.
* A plug-in catalogue, signed plug-ins and per-plug-in permissions beyond the fixed rules above.

## For plug-in authors

Plug-ins can be written in any language that compiles to WebAssembly. The AniCraft repository
has a small Rust kit and two complete sample plug-ins, Star Field (a generator) and Jitter
Selection (a selection effect). The plug-in interface, the manifest that describes a plug-in's
settings, the exact limits and the frame-script requests are documented in
[the plug-in API on GitHub](https://github.com/storytold/AniCraft/blob/main/docs/plugins.md).

You can test a plug-in without installing it: point AniCraft at a folder of `.wasm` files with
the `ANICRAFT_PLUGINS` environment variable, then use **Commands › Reload Plug-ins** after each
build. Every command a plug-in can run is listed in [the command reference](reference/commands.md).
