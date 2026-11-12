# The live control port

`anicraft --control <port>` opens the AniCraft window as usual and also listens on a port on this
computer, so an assistant (or a script) can edit the open document while you watch. Every edit
appears in the window at once and undoes with Ctrl+Z, like your own. The status bar says
"Remote control on (port N)" while it is open. ADR 0070 records the design.

Most people never speak this protocol themselves: AI assistants reach it through
`anicraft-cli mcp --bridge <port>` (see `docs/mcp.md`).

```sh
anicraft --control 9878                 # 9878 by convention
ANICRAFT_CONTROL_PORT=9878 anicraft     # the same; --control wins over the variable
```

## Transport

* TCP on `127.0.0.1` only.
* One JSON object per line (UTF-8, `\n`; a `\r` before it is ignored), each way.
* Connections stay open; requests on one connection are answered in order.

## Authentication

Each run makes a fresh token: 64 hex digits from the operating system's random numbers. The app
writes it to `control.json` in AniCraft's settings folder:

| System | File |
|---|---|
| Windows | `%APPDATA%\AniCraft\control.json` |
| macOS | `~/Library/Application Support/AniCraft/control.json` |
| Linux | `$XDG_CONFIG_HOME/anicraft/control.json` (default `~/.config/anicraft/`) |

```json
{"port": 9878, "token": "3f1c…", "pid": 12345, "version": "0.2.1"}
```

On macOS and Linux the file is readable only by you; on Windows the settings folder already is.
It is replaced when AniCraft starts with `--control` and removed when it closes. A program running
as you finds the token there, with nothing to copy into its settings.

If `ANICRAFT_CONTROL_TOKEN` is set, it is the token instead (set it for both the app and the
client: CI, power users).

Every request carries `"token"`. A missing or wrong token gets an error and the connection
closes before anything runs.

## Requests and replies

```json
{"id": 1, "token": "…", "method": "hello"}
{"id": 1, "ok": true, "result": {"app": "AniCraft", "version": "0.2.1", "protocol": 1}}
```

`id` is any JSON value and comes back as it was. A reply is `{"id", "ok": true, "result"}` or
`{"id", "ok": false, "error": "message"}`.

| Method | Params | Result |
|---|---|---|
| `hello` | | `{app, version, protocol}` |
| `tools.list` | | the tools, exactly as MCP `tools/list` lists them (`docs/mcp.md`) |
| `tools.call` | `{name, arguments}` | exactly what MCP `tools/call` returns: `{content: […], isError}` |

`tools.call` runs the same code as the headless MCP server. A tool that fails (a command's error)
is a result with `"isError": true`; a malformed call (an unknown tool, a missing `name`) is an
`"ok": false` reply.

```json
{"id": 2, "token": "…", "method": "tools.call", "params": {"name": "anicraft_run", "arguments": {"command": "draw.rect", "params": {"x": 10, "y": 10, "w": 50, "h": 50}}}}
{"id": 2, "ok": true, "result": {"content": [{"type": "text", "text": "{\"frame\": 0, \"result\": {…}, \"revision\": 3, \"scene\": 0}"}], "isError": false}}
```

## When calls run

The window runs calls at the start of a frame, so an edit shows in that frame. It never takes
keyboard focus and doesn't need to be the front window. An edit made while the start screen is showing closes it, so you see
the document it changed.

While you are dragging (or a plug-in dialog is previewing), calls that could change the document
wait until you let go, so your drag and the assistant's edit stay separate undo steps. Calls that
only read run at once. If you were typing in a field, your change is kept as its own undo step
first. A call that waited 60 seconds is not run, and its reply says so.

Commands that would lose work or replace a file still need `"confirm": true` (ADR 0004).

## Limits

| | |
|---|---|
| Line length | 4 MiB; a longer line gets an error and the connection closes |
| Connections | 16 at once; another gets an error line and is closed |
| Waiting | 60 s per call |
| First line | must be a JSON object with a string `"method"`, or the connection closes (this also refuses a web page's request to localhost) |

## Trying it by hand

With AniCraft open with `--control 9878`, in PowerShell:

```powershell
$t = (Get-Content "$env:APPDATA\AniCraft\control.json" | ConvertFrom-Json).token
$c = New-Object Net.Sockets.TcpClient 127.0.0.1, 9878
$w = New-Object IO.StreamWriter $c.GetStream(); $r = New-Object IO.StreamReader $c.GetStream()
$w.WriteLine("{""id"":1,""token"":""$t"",""method"":""hello""}"); $w.Flush(); $r.ReadLine()
```
