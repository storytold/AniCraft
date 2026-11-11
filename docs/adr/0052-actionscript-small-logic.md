# ADR 0052: ActionScript small logic (tier 2.5)

* Status: accepted (2026-11-11), built
* Relates to: ADR 0013 (ActionScript compatibility: tiers 1 and 2, the plan), ADR 0024 (the
  action IR and the pure player), ADR 0028 (HTML canvas export and `player.js`), roadmap M6

## Context

ADR 0013's tier 2 runs the ActionScript 3.0 timeline subset by lowering scripts to a flat list
of timeline actions ahead of time. Item 2 of that ADR's context, "small logic" (variables,
`if`, loops, `trace`, setting and reading instance properties, simple functions), can't be said
that way: a loop's count, a condition or a computed frame depend on the movie as it plays.
Tier 2 left out variables and conditions, reading properties, `removeEventListener`,
enter-frame and keyboard events, functions on other frames, `e.target` and `Math.random`.

## Decision

* **A script IR, run by an evaluator, instead of a flat action list.** Small logic can't be
  lowered to actions ahead of time (a loop's count, a condition, a computed frame depend on
  the movie), so a compiled script is now a small typed tree of statements and expressions
  (`anicraft_actions::as3::ir`: `Stmt`, `Expr`, `LValue`, `Place`, `Function`) and
  `as3::run` evaluates it against a `World` trait that the player implements. It is data, not
  code: no dynamic evaluation, no `eval`, no reflection. Timeline control still ends in the
  same `Action`s (an `Act` statement builds one with the frame and scene computed), so hand-made
  actions, plug-in frame scripts and ActionScript keep one runtime. The tier-2 `ScriptOp`
  list is gone; `script.check` returns `steps` (a count), `program` (the IR as JSON) and
  `uses`. Both the evaluator and its interface stay in the L0 crate: it knows values and
  names, not documents.
* **Values and types.** ActionScript's primitives (`undefined`, `null`, Boolean, Number,
  String), with ECMA-262's operators exactly: `+` joins when either side is text, `==` and
  `<` as §11.8–11.9 (text compares by UTF-16 units), ToNumber as browsers read text (decimal,
  `Infinity`, `0x`/`0o`/`0b`), ToString as `Number.toString`. Typed variables coerce on every
  write: `int` and `uint` as ToInt32/ToUint32 (so `2147483647 + 1` wraps), Number, Boolean,
  String (`null` stays `null`), untyped keeps anything; each starts at its type's default (0,
  NaN, false, null, undefined). `int(…)`, `uint(…)`, `Number(…)`, `String(…)`, `Boolean(…)`
  cast. Other types (`Array`, `MovieClip`, objects) are problems.
* **Where variables live, as ActionScript 3.0 timelines keep them.** A `var` at a frame
  script's top level is a member of the timeline: one table per timeline *instance*
  (`PlayerState.vars`, by instance path), shared by every frame script of that timeline, kept
  while the instance stays on the stage and gone when it leaves (a clip that comes back starts
  afresh), as an instance is constructed again. Every variable any frame of the timeline
  declares exists from the start with its default, so frame 1 can read what frame 5 declares
  (`null` until it runs). `var` inside a function is that call's (hoisted, numbered slots).
  The main timeline's scenes share one table, as they are one timeline.
* **Linking across frames.** A script compiles on its own (the cached compile stays a pure
  function of its text) and lists the names it takes from elsewhere (`Compiled::free`). A
  timeline's **declarations** (`Decls`: its scripts' functions and variable types, the first
  of each name in frame order, top layer first) are built from its compiled scripts; a script
  whose free names the timeline doesn't declare, or that declares a function or a variable
  type another frame already declares differently, doesn't run (whole, as before), and Test
  Movie's first lines, the Script view, `script.set` and `script.list` say which name and
  where. So functions on **any** frame of the same timeline can be called (ActionScript makes
  them methods of the timeline), not only earlier ones. Calls go by name through the
  declarations at run time.
* **Statements.** `if`/`else`, `for (init; cond; next)`, `while`, `do … while`, `break` and
  `continue` (no labels), `return` with a value (coerced to the declared result type), calls
  with parameters (typed, defaults written out) and recursion, `++`/`--` and `+= -= *= /= %=`
  on variables and instance properties, `&&`/`||`/`?:`, `Math.min/max/abs/floor/ceil/round/
  random`, `Math.PI` and `Math.E`, the `Keyboard` key constants. Not yet: `switch`, `for in`/
  `for each`, arrays and objects, `new`, bitwise operators, string methods, nested function
  declarations, closures, `try`/`throw`.
* **Reading properties.** `x`, `y`, `alpha`, `visible`, `rotation`, `scaleX/Y`, `name`,
  `mouseX/Y` of named instances, the timeline (`this`, `parent`, `root`) and the event's
  targets; `currentFrame` (1-based), `totalFrames` (every scene's for the main timeline),
  `currentLabel` (the label at or before the playhead, or `null`) of movie clips and the main
  timeline. A value a script set reads back as set; one the timeline gives is decomposed from
  its transform. Values are the transform's `f32` numbers written shortest (`0.1`, not
  `0.10000000149011612`), which is also what the HTML page reads from its JSON, so both say
  the same. `mouseX/Y` keep the pointer's last place after it leaves, as in ActionScript.
  Reading an instance that isn't there **stops the script there** (ActionScript throws), unlike
  an action on a missing instance, which still writes an Output line and goes on (tier 2).
* **Listeners.** `removeEventListener` (by event, timeline and function name; an inline
  function can't be removed). `Event.ENTER_FRAME` on the timeline, a movie clip, a button or
  the stage: every frame, playing or stopped, **after playheads move and before that frame's
  scripts run** (ActionScript dispatches `enterFrame` before frame scripts), in the order the
  listeners were added. `KeyboardEvent.KEY_DOWN` and `KEY_UP` on the **stage** (where keys go
  when nothing has focus, as in a published movie) with `e.keyCode`; elsewhere they would
  never fire, so they are ignored and noted. `e.target` and `e.currentTarget` are the instance
  listened to (the stage for keys); with no bubbling yet, a part inside a clip is not a
  target of its own. `e == …` between instances compares them (`e.currentTarget == btn_a`).
  The event passes on to a function whose parameter is typed `…Event`. Mouse listeners on
  the stage and other event fields (`stageX`, `charCode`, `type`) are not yet.
* **Deterministic random.** `Math.random()` draws from a seeded generator in the player state
  (Marsaglia's xorshift128, 53 bits per number), so the same document, seed and events give
  the same movie, tested. `start_seeded(doc, seed, …)`; `control.test_movie` takes `seed`
  (default 0); the app's Test Movie seeds from the clock on each start (screenshots use 0), and
  the HTML page from `crypto.getRandomValues`.
* **Fuel, and how a script stops.** Every statement and expression costs one unit of fuel, a
  loop's turn one more, and making text one more per 64 characters. A frame script or one
  listener call gets 100,000; a step 1,000,000 over all of them. A script that runs out stops
  where it is with one Output line ("Script on frame 3 of the main timeline stopped: it ran
  more than 100000 steps (an endless loop?)"), and so does one that hits a run-time error
  ("… stopped: line 4: no instance named “ghost” here"); what it did before stays done, as an
  ActionScript error leaves it, and the movie plays on. When the step's fuel is used up, the
  step stops running actions with the existing message. Calls nest at most 16 deep, text is
  at most 16,384 UTF-16 units, an Output line 1,000 (cut at a whole character); a binary
  operator chain counts toward the 64-level nesting limit, so evaluating never recurses
  without bound (a test runs the deepest allowed code on a 512 KiB stack). Measured (this
  build's optimised dev profile): a 100,000-fuel loop runs in about 2 ms, so a step's whole
  budget is about 20 ms.
* **Test Movie keyboard.** The movie surface takes the keyboard when the window opens and when
  it is clicked; its keys (letters, digits, arrows, Space, Enter, Tab, Backspace, F1–F12, …)
  go to the movie as ActionScript key codes, Tab and the arrows included; shortcuts with Ctrl
  (⌘) or Alt still reach the app (Restart is Ctrl+Enter) and Esc closes the window. The window
  says "Keys go to the movie" (or "Click the movie to give it the keyboard") when the movie
  listens for keys. `control.test_movie` events take `{type: key_down|key_up|key, key}` with a
  code, a letter or a `Keyboard` name.
* **HTML canvas export.** A script that runs goes out as its IR (`{"as": {"b": body, "i":
  inline functions}}`), each timeline that runs scripts with its declarations (`fn`, `vt`),
  and `player.js` mirrors the evaluator line for line, using JavaScript's own operators for
  ECMAScript's. Parity tests play the same scripted documents in both, step by step, comparing
  output, state (variables and stage listeners included) and display list, and compare
  `Math.random()` number for number at several seeds. Keys reach the page's movie from the
  window (arrows and Space stop scrolling the page when the movie listens for keys).
* **Not in tier 2.5:** `switch`, `for in`/`for each`, arrays, objects and `new`, string
  methods, bitwise operators, closures and nested functions, event bubbling and `e.target`
  inside a clip, mouse listeners on the stage, other event fields, `getChildByName`, `stage`
  properties, sound, timers, `navigateToURL` (still noted), and ActionScript 1.0 and 2.0. The
  XFL import report counts a script that compiles as running before the timeline is linked.

## Consequences

* Scripts from Animate files that count, branch, loop, read where things are or answer the
  keyboard now play in Test Movie and the HTML page; the rest are still kept, not run, with
  the reason in place.
* The player and `player.js` now share an evaluator's semantics, not just a list of actions:
  every change to the IR or the evaluator needs both sides and a parity test.
* Exports other than HTML canvas, the stage and `evaluate` are unchanged: scripts never touch
  the document (ADR 0013's "time stays a function").

