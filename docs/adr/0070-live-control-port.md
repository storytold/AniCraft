# ADR 0070: A live control port, so assistants edit the open window

* Status: accepted (2026-11-12)
* Relates to: ADR 0004 (destructive actions), ADR 0005 (typography), ADR 0006 (platforms),
  ADR 0040 (interface languages), `docs/mcp.md`, `docs/control-protocol.md`

## Context

`anicraft-cli mcp` gives AI assistants the engine's commands as five MCP tools, but it edits a
document inside its own process, with no window. To see the result the person saves, then opens
the file in the app. Every edit is invisible until then, and the assistant and the person can
never work on the same document at once.

The sister apps (VectorCraft, CadCraft, FilmCraft, EffectCraft) already let an assistant drive the
open window through a TCP port on `127.0.0.1`, with a line-per-message JSON protocol and an MCP
"bridge" that forwards to it. AniCraft should behave the same way.

One lesson from FilmCraft: its token was an environment variable, which a desktop shortcut's
process received and an MCP server started by the assistant's desktop app did not. Authentication
must not depend on an environment variable reaching two unrelated processes.

## Decision

1. **`anicraft --control <port>`** (also `ANICRAFT_CONTROL_PORT`; the flag wins) opens a control
   port on `127.0.0.1` only. 9878 is the convention (after EffectCraft's 9877). A bad port is an
   error before any window opens. If the port can't be opened (in use), the window opens anyway
   and the status bar says why.
2. **The protocol is small** (`docs/control-protocol.md`): one JSON object per line each way,
   persistent connections, answered in order. Methods: `hello`, `tools.list`, `tools.call`.
   Replies are `{"id", "ok": true, "result"}` or `{"id", "ok": false, "error"}`.
3. **One dispatcher for live and headless.** The app depends on the `anicraft-cli` library and
   runs its `mcp::call_tool` and `mcp::tools` on the window's session. `call_tool` is generic over
   a small `Host` trait (the session, and how a command runs), so the app runs commands through
   its own follow-up (`App::announce`: the status line, the view a new document resets, Open
   Recent), as for the person's own edits.

   *Considered:* a new crate for the dispatcher, used by both. It would have to be L4 (it needs
   the engine and the renderer), and the CLI (L4) may not depend on a crate in its own layer, so
   the CLI would have to move up beside the shell. The shell (L5) depending on the CLI library
   (L4) fits the layering as it is, adds no crate, and the CLI library has no dependency the shell
   doesn't already have (except `getrandom`, below). The CLI's library already was the MCP server;
   it now also holds the control port's protocol, its listener threads and the bridge client
   (`crates/cli/src/control.rs`), none of which touches the UI toolkit, so they are tested there.
4. **Threads, and the frame.** One listener thread and one thread per connection (at most 16). A
   tool call goes to the UI thread through a channel with a reply sender; the connection thread
   wakes the window (`request_repaint`). The frame loop runs waiting calls at the start of the
   frame, before anything draws, so their edits show in the same frame (AGENTS.md "edits are
   live"). The window never takes focus. An edit made behind the start screen (ADR 0059)
   closes it, so the person sees the document change.
5. **The person's own gestures.** An edit from a tool must not join the person's undo step. While
   the person is dragging (pointer held, a stage drag, or a plug-in dialog previewing), calls that
   can change the document wait (calls that only read don't). A call that waited 60 s is refused
   without running, and the client is told so. An open gesture that is only typing (a property
   field still focused) is committed first, as its own undo step, as clicking elsewhere would do;
   waiting for that could block an assistant indefinitely.

   *Considered:* committing the gesture mid-drag. The rest of the drag would then record one undo
   step per frame, so we wait instead.
6. **Authentication: a token file, no setup.** Each run with `--control` makes a fresh 32-byte
   token from the operating system's random numbers (`getrandom`, MIT OR Apache-2.0, already in
   the tree) and writes `control.json` (`port`, `token`, `pid`, `version`) to the AniCraft settings
   folder, owner-only on macOS and Linux (`%APPDATA%` is already private on Windows). It is
   replaced at start and removed on exit if it still holds this run's token. Every request carries
   the token; a missing or wrong token is refused and the connection closed before anything runs.
   `ANICRAFT_CONTROL_TOKEN`, when set, is the token on both sides (CI, power users).

   The CLI can't use the shell's `platform::config_dir`, so it has its own copy of the folder rule
   (`control::settings_dir`) and a shell test keeps the two equal.

   *Considered:* no authentication (VectorCraft, CadCraft, EffectCraft). Any local process, and a
   web page able to reach localhost, could then edit and save files as the person. *Considered:*
   an environment-variable token only (FilmCraft): see Context.
7. **Hardening, as in the sister apps.** Every line must be a JSON object with a string `method`;
   anything else gets one error line and the connection closes. That also refuses a web page's
   `fetch` to localhost, which starts with an HTTP request line. Lines are capped at 4 MiB. Writes
   time out after 10 s so a client that stops reading can't hold a thread.
8. **Safety rules unchanged.** Calls run through `Session::execute`, so `"confirm": true` (ADR 0004)
   is needed exactly as headless, unsaved-work guards and crash recovery see the edits as the
   person's, and every edit undoes with Ctrl+Z.
9. **`anicraft-cli mcp --bridge <port | 127.0.0.1:port>`** is the MCP server with a backend that
   forwards `tools/call` to the app; `initialize` and `tools/list` are answered locally (the same
   tool list), so the server starts and lists its tools before AniCraft is open. It connects on
   the first call and returns an error that says how to start the app when it can't. It reconnects
   after a break (an app that quit is noticed before the next request is sent). An edit whose reply
   was lost is never sent again (the app may have made it); a call that only reads is tried once
   more. Hosts other than this computer are refused.
10. **Visible.** While the port is open the status bar says "Remote control on (port N)" (through
    `tr!` and `theme::Type`), with a tooltip saying what that means.

## Consequences

* The shell now builds the CLI library (small; nothing new for the web build, which leaves the
  control port out).
* The tool behaviour can't drift between live and headless: one function serves both, and a test
  checks that a call through the port answers as `call_tool` does.
* Remote edits arrive at most once per frame batch; a long `anicraft_run_script` blocks the window
  while it runs, as a long menu command would.
* Not yet: a screenshot tool that returns the window itself (the app already captures itself for
  `--screenshot`).
