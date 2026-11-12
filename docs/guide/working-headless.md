# Working headless

Everything you do in AniCraft is a named command, and the same commands run without a window.
This chapter is for technical artists and pipeline TDs: rendering from the command line, batch
jobs, scripting edits as JSON, and connecting AI assistants.

On macOS, press ⌘ where this guide says Ctrl, and ⌥ where it says Alt.

## The command-line tool

`anicraft-cli` comes with AniCraft:

* **Windows**: `anicraft-cli.exe` sits next to `anicraft.exe` in the install folder. It isn't
  added to your `PATH`, so call it by its full path or add the folder yourself.
* **macOS**: it is inside the app at `AniCraft.app/Contents/MacOS/anicraft-cli`, and also a
  separate download, `anicraft-cli-<version>-macos-universal.zip`, on the
  [releases page](https://github.com/storytold/AniCraft/releases).
* **Linux**: the deb and rpm packages install it as `/usr/bin/anicraft-cli`; the tarball has it
  next to `anicraft`.

Run it with no arguments, or with `--help`, to see every subcommand. `anicraft-cli --version`
prints the version.

Two rules hold everywhere:

* Each subcommand takes a document path, or `--sample` for the built-in sample animation. The
  positional document must be a `.anicraft` file (to work on an Animate file, see
  [Converting Animate files](#converting-animate-files)).
* **Frame flags on the command line are 1-based**, as on the timeline: `--frame 1` is the first
  frame. Frame numbers inside JSON params are 0-based, as everywhere in the engine.

## Rendering

| Subcommand | What it writes |
|---|---|
| `info <file>` | A JSON description of the document: stage, scenes, layers, keyframes, library. |
| `render <file> --out frame.png [--frame N] [--scale S]` | One frame as a PNG. |
| `render-seq <file> --out-dir dir [--from A] [--to B] [--scale S]` | A numbered PNG sequence. |
| `gif <file> --out anim.gif [--from A] [--to B] [--scale S] [--overwrite yes]` | An animated GIF. |
| `apng <file> --out anim.png [...]` | An animated PNG, full colour with real transparency. |
| `webp <file> --out anim.webp [...]` | A lossless animated WebP. |
| `new --out file.anicraft [--width W] [--height H] [--fps F]` | A new, empty document. |

`apng` and `webp` take the same range and scale flags as `gif`, plus `--loop forever` (the
default) or `--loop N` for how many times it plays, and `--background yes` to draw the stage
colour instead of transparency.

Every frame is drawn by the same renderer the app uses for its exports, so what you render here
matches File › Export exactly.

```sh
anicraft-cli render shot010.anicraft --frame 24 --out shot010_f24.png
anicraft-cli render-seq shot010.anicraft --out-dir frames/shot010 --scale 0.5
anicraft-cli gif --sample --out preview.gif --scale 0.5
anicraft-cli webp shot010.anicraft --out loop.webp --loop 3
```

### Replacing files

Exports never overwrite a file silently. If the output already exists, the command fails and
says so ("Replace existing file? f.png already exists…"). Pass `--overwrite yes` to `gif`,
`apng` and `webp` when replacing is what you want. `render` has no such flag: delete the old file
first, or run the export command itself with `"confirm": true` (see
[Destructive commands need "confirm"](#destructive-commands-need-confirm)):

```sh
anicraft-cli exec shot010.anicraft file.export_png '{"path":"f24.png","frame":23,"confirm":true}'
```

## Running any command

Every menu item is a command with an id, and every command can be run from the command line.

`anicraft-cli commands` prints the whole registry as JSON: each command's `id`, `label`, `menu`,
default `shortcut`, a short `params` description, and whether it changes the document. The same
list, readable, is in [the command reference](reference/commands.md).

The `menu` field tells you where the command lives in the app. `"File/Export"` is
**File › Export**, `"Modify/Transform"` is **Modify › Transform**, and an empty menu means the
command has no menu item (it is used by panels, tools or scripts). For example,
`timeline.insert_keyframe` is **Insert › Timeline › Keyframe** (**F6**), and `file.export_html`
is **File › Export › Export HTML Canvas…**.

### One command: exec

```sh
anicraft-cli exec <file|--sample> <command.id> ['<json-params>'] [--save out.anicraft]
```

The result prints as JSON. Add `--save` to write the changed document.

```sh
anicraft-cli exec --sample draw.rect '{"x":10,"y":10,"w":80,"h":40}' --save box.anicraft
anicraft-cli exec shot010.anicraft file.export_html '{"path":"web/shot010.html"}'
anicraft-cli exec shot010.anicraft file.export_video '{"path":"shot010.mp4"}'
```

Video export streams frames to your own copy of ffmpeg. AniCraft looks for it in the `ffmpeg`
param, then the `ANICRAFT_FFMPEG` environment variable, then your `PATH`.

On Windows PowerShell, quoting JSON on the command line is awkward. Put the steps in a script
file instead (next section).

### Several commands: run

A script is a JSON array of steps, each with a `cmd` and optional `params`:

```json
[
  {"cmd": "layer.new", "params": {"name": "Title"}},
  {"cmd": "draw.rect", "params": {"x": 40, "y": 40, "w": 400, "h": 120, "fill": "#dac604"}},
  {"cmd": "timeline.insert_keyframe", "params": {"frame": 23}},
  {"cmd": "file.export_gif", "params": {"path": "title.gif"}}
]
```

```sh
anicraft-cli run shot010.anicraft title.json --save shot010_titled.anicraft
```

Steps run in order. The first one that fails stops the run, and the error names the step. The
results of all the steps print as a JSON array.

### Publishing a document

If a document has **File › Publish Settings** set up, one command writes every format it has
switched on, to the output names saved in the document (relative to the document's folder):

```sh
anicraft-cli exec shot010.anicraft file.publish '{"confirm": true}'
```

`confirm` lets it replace files left by an earlier publish. The result is the same report the
app shows: what each format wrote, skipped or why it failed.

## Batch jobs for pipelines

Because each call is a plain process with JSON in and out, AniCraft fits into any build system,
render farm or shell loop. Exit status is 0 on success and 1 on any error, with the message on
standard error, so a failed step stops your pipeline.

Render every document in a folder to a half-size GIF:

```sh
for f in shots/*.anicraft; do
  anicraft-cli gif "$f" --out "out/$(basename "$f" .anicraft).gif" --scale 0.5 --overwrite yes
done
```

The same in PowerShell:

```powershell
Get-ChildItem shots\*.anicraft | ForEach-Object {
  & anicraft-cli gif $_.FullName --out "out\$($_.BaseName).gif" --scale 0.5 --overwrite yes
}
```

Renders are deterministic: the same document and flags give the same pixels on every machine.

### Converting Animate files

The positional document must be a `.anicraft` file, but the `file.open` command reads `.fla`,
`.xfl` and XFL folders. Start from the sample, open the Animate file over it, and save:

```sh
anicraft-cli exec --sample file.open '{"path":"intro.fla"}' --save intro.anicraft
```

The result includes what was imported, kept or left out, as the app's import report does. See
[Opening Animate files](opening-animate-files.md) for what comes across.

### Plug-ins on the command line

Headless runs load no plug-ins unless you ask, so a script gives the same result on every
machine. Add `--plugins <folder>` to any subcommand (or set `ANICRAFT_PLUGINS`) to load the
`.wasm` files in that folder first, then run them with `plugin.run`:

```sh
anicraft-cli exec --sample plugin.run '{"id":"org.anicraft.star-field","params":{"count":200}}' \
    --plugins my-plugins --save stars.anicraft
```

See [Plug-ins](plugins.md).

## A live session: serve

`anicraft-cli serve [file|--sample]` keeps one document open and reads one JSON request per
line on standard input, answering one line each on standard output. Tools that drive AniCraft
interactively (a game engine exporter, a custom panel in another app) use it.

```text
{"id": 1, "cmd": "timeline.insert_keyframe", "params": {"frame": 10}}
{"id": 1, "ok": true, "result": {}, "revision": 7, "frame": 0}
```

The first line out is `{"ready": true, ...}`. A request with `"cmd": "commands"` returns the
registry, a failed command answers `"ok": false` with an `error`, and `{"cmd": "quit"}` ends the
session. Undo and redo are commands too (`edit.undo`, `edit.redo`).

## Destructive commands need "confirm"

In the app, actions that would lose work for good ask first: discarding unsaved changes,
replacing a file, deleting an autosaved copy or an installed plug-in. Headless, there is nobody
to ask, so those commands refuse until their params include `"confirm": true`:

```text
error: step 2: file.new: Discard unsaved changes? “Untitled” has changes that are not saved.
They will be lost, and its undo history is cleared. (pass "confirm": true to proceed)
```

Add `"confirm": true` only when you mean it. Ordinary edits, such as deleting a layer, never need
it: they can be undone with `edit.undo` in the same session.

## AI assistants (MCP)

`anicraft-cli mcp` is a Model Context Protocol server. An AI assistant that supports MCP can
then open a document, draw, animate, look at frames and export, through the same commands the
app uses.

```sh
anicraft-cli mcp                    # a new, empty document
anicraft-cli mcp scene.anicraft     # open a document
anicraft-cli mcp --sample           # the sample animation
```

To register it with Claude Code:

```sh
claude mcp add anicraft -- /path/to/anicraft-cli mcp
```

Most other clients take an entry like this (use the full path to `anicraft-cli`):

```json
{
  "mcpServers": {
    "anicraft": {
      "command": "/path/to/anicraft-cli",
      "args": ["mcp", "/path/to/scene.anicraft"]
    }
  }
}
```

The assistant gets five tools:

| Tool | What it does |
|---|---|
| `anicraft_commands` | Lists commands, optionally filtered by words such as `tween` or `export`. |
| `anicraft_run` | Runs one command with its params. |
| `anicraft_run_script` | Runs several commands in order, stopping at the first error. |
| `anicraft_document` | Describes the document: stage, scenes, layers, keyframes, library, selection. |
| `anicraft_render` | Returns a frame as a PNG picture, exactly as exported. |

Things to know:

* Every edit is checked and can be undone, as in the app.
* Commands that would lose work or replace a file still need `"confirm": true`. The server tells
  the assistant to check with you before confirming.
* The assistant reads and writes files with your permissions, at the paths it chooses. Only
  connect assistants you trust with those files.
* The server works on its own copy of the document, not on the window you have open. Save from
  the assistant, then open the file in AniCraft.

The full details are in
[the MCP notes on GitHub](https://github.com/storytold/AniCraft/blob/main/docs/mcp.md).

## This guide as a web page

`anicraft-cli guide --out <folder>` writes this user guide as HTML pages into a folder, with its
pictures, so you can read it offline or host it next to your studio's own documentation:

```sh
anicraft-cli guide --out docs/anicraft-guide
```

## Performance check

`anicraft-cli bench` builds a large generated document and times editing, undo, rendering,
saving and autosave on your machine. Run it on a release build; `bench --check yes` compares the
default scenario with AniCraft's published budgets and fails if one is missed.
