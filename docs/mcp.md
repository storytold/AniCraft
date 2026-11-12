# AniCraft for AI assistants (MCP)

`anicraft-cli mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server. Any
MCP client (an AI assistant, an editor, a script) can then open a document, draw, animate,
look at frames and export through the same commands the app uses.

```sh
anicraft-cli mcp                      # a new, empty document
anicraft-cli mcp scene.anicraft       # open a document
anicraft-cli mcp --sample             # the sample animation
anicraft-cli mcp --bridge 9878        # the AniCraft window you have open (below)
```

## Headless or live

Without `--bridge` the server edits a document of its own, with no window: good for pipelines and
batch work. Save it (`file.save`) to keep the result.

With `--bridge <port>` the same tools act on the AniCraft window you have open, started with
`anicraft --control <port>`: every edit appears there at once, and Ctrl+Z undoes it
(`docs/control-protocol.md`, ADR 0070). The server starts even when AniCraft isn't open (an
assistant starts its servers when it starts) and connects on the first tool call; until AniCraft is
open with remote control, the tools answer with how to start it. Close and reopen AniCraft whenever
you like: the next call reconnects, with no need to restart the assistant. It finds the run's token
in `control.json` in AniCraft's settings folder, so there is nothing to copy.

The server talks MCP's stdio transport: JSON-RPC 2.0, one message per line on stdin and stdout.
It speaks protocol revisions 2025-06-18, 2025-03-26 and 2024-11-05.

## Connecting a client

Most clients take a server entry like this one (use the full path to `anicraft-cli`):

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

For live editing, start AniCraft with `--control 9878` (a desktop shortcut is handy) and use:

```json
{
  "mcpServers": {
    "anicraft": {
      "command": "C:\\path\\to\\anicraft-cli.exe",
      "args": ["mcp", "--bridge", "9878"]
    }
  }
}
```

## Tools

| Tool | What it does |
|---|---|
| `anicraft_commands` | Lists the commands (id, label, menu, shortcut, params), optionally filtered by words such as `tween` or `export`. |
| `anicraft_run` | Runs one command with its params. |
| `anicraft_run_script` | Runs several commands in order; stops at the first error and says which step failed. |
| `anicraft_document` | Describes the document: stage, scenes, layers and keyframes, library, and the current scene, frame, layer and selection. |
| `anicraft_render` | Returns a frame of the current scene as a PNG picture, exactly as exported. |

The assistant sees one tool per job, not one per command: the command list is long and grows
with every release, and `anicraft_commands` lets it find what it needs. Frames in params are
0-based, as everywhere in the engine (the CLI's own `--frame` flags are 1-based, as in the
timeline).

## Safety

* Everything runs through the engine's commands, so params are checked and every edit can be
  undone with `edit.undo`.
* Commands that would lose unsaved work or replace a file refuse until their params include
  `"confirm": true` (ADR 0004). The server's instructions ask the assistant to check with you
  before confirming.
* Commands read and write files with your permissions, at the paths the assistant gives (open,
  save, import, export), as with the CLI. Only connect clients you trust with those files.
* A failing command comes back as a tool result marked as an error, so the assistant can read
  the message and try again; it never stops the server.
* Live (`--bridge`): an edit whose answer was lost (AniCraft closed mid-call) is never sent again,
  since it may have been made; the assistant is told to check with `anicraft_document`. While you
  are dragging, the assistant's edits wait until you let go, so the two stay separate undo steps.
