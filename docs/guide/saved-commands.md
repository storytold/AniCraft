# Saved commands

Do the same few edits over and over? Keep them as a **saved command**: pick the steps in the
History panel, save them with a name, and run them again on any selection from the **Commands**
menu or with a key of your own. A saved command runs as **one undo step**, so a single Ctrl+Z
takes it all back. This works as Save As Command does in Animate.

## Saving steps as a command

1. Make the edits once on something selected (say, rotate it, fade it to 40 % and move it).
2. In the **History** panel (Window › History), **Ctrl+click** each step you want to keep.
   **Shift+click** picks every step between the last one you picked and this one. A plain click
   still goes back to that step, as before. You can also right-click a step and choose
   **Pick for Save As Command**.
3. Click **Save As Command…** at the bottom of the panel (or right-click a step and choose it
   there).
4. Give it a name and click **Save**.

The dialog lists the picked steps in order. Some steps name a particular layer or object of this
document; where the command can work on the selection (or the active layer) instead, that's what
the saved command does, and the dialog says so ("acts on the selection"). A step that only makes
sense in this document, such as moving particular path points, placing a particular Library
item or using the clipboard, can't be kept: the dialog says which step and why, and **Save**
stays off until you Ctrl+click that step to take it out of the picks.

Selecting things and moving the playhead aren't steps, so a saved command always works on what
is selected when you run it.

## Running a saved command

Your saved commands are at the end of the **Commands** menu, with their keys. Choose one and it
runs on the selection. If a step can't run (nothing is selected that it needs, say, or the layer
is locked), the command stops there, **nothing is changed**, and the status line names the step
that stopped it.

**Commands › Run Command…** runs a saved command file without adding it to your list, for
example one a colleague sent you.

## Managing saved commands

**Commands › Manage Saved Commands…** lists your commands in their menu order. For each one:

* **Run** runs it on the selection.
* **↑** and **↓** move it up or down the Commands menu.
* **Shortcut…** opens Keyboard Shortcuts on it, waiting for the keys you press. You can also find
  it there yourself, under Commands. Keys you give a saved command follow it when you rename it.
* **Rename…** renames it.
* **Delete…** deletes it, after asking (undo can't bring a file back).

**Add from File…** copies a saved command file into your commands.

## Sharing saved commands

Each saved command is a small JSON file in the `Commands` folder of your settings folder
(Preferences › About shows where). Send the file to share it; whoever receives it uses
**Run Command…** or **Add from File…**. The file's name is the command's name.

Saved commands can do what [plug-ins](plugins.md) may do: change the document. They can't open
or save files, use the clipboard, change your preferences, run plug-ins or run other saved
commands, so a file from someone else can't do any of that either. Files that aren't saved
commands, or that ask for something they may not do, are refused with the reason.

## On the command line and for AI assistants

The same commands work [headless](working-headless.md):

```sh
anicraft-cli run scene.anicraft "Turn and Fade.json" --save out.anicraft
anicraft-cli exec scene.anicraft macro.run '{"path": "Turn and Fade.json"}'
```

`anicraft-cli run` takes either a saved command or a plain list of `{cmd, params}`. Assistants
using the MCP server call `macro.run` through `anicraft_run`. `macro.history`, `macro.save`,
`macro.list`, `macro.rename`, `macro.move`, `macro.delete` and `macro.import` are in the
[command reference](reference/commands.md).

## What isn't there yet

Replaying picked steps without saving them, editing a saved command's steps in the app, saved
commands that ask for a value when they run, and saved commands in the browser version.
