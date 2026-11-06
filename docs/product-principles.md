# Product principles

These two standing rules apply to every proposal, review and commit.

## 1. Design like a principal engineer

Before building anything, ask: **"Is this how a principal engineer would do it?"** If not, use
the better method, or ask the project lead when the choice changes the product. A principal
engineer:

* **Measures before optimizing**, and records the numbers (`docs/performance.md`).
* **Prefers correctness by construction** over correctness by discipline. Examples: snapshot
  undo on a persistent document instead of hand-written inverse commands (ADR 0001), and one
  command registry for UI, CLI and agents.
* **Chooses the simplest design that holds at 100× scale**: virtualized lists, background
  rendering, bounded memory.
* **Writes the decision down** (an ADR for anything hard to reverse) with the options
  considered.
* **Ships in small, green, reversible steps** with tests that would have caught the bug.
* **Pushes back on a requirement when its literal form hurts users**, and explains the
  better alternative. ADR 0004 is an example.

## 2. Build for artists and creatives

Our users animate 2D vector art, many of them coming from Adobe Animate. They expect a
**professional, fast, reliable and easy** tool:

| Quality | What it means here |
|---|---|
| Professional | Familiar layout (tools left, stage centre, timeline bottom, panels right), familiar names and shortcuts (F5/F6/F7, F8, Ctrl+Z…), exact output. |
| High performance | Never block the UI. Playback at document fps. Large files open and edit instantly (budgets in `performance.md`). |
| Reliable | Never crash, never lose work. Unsaved-change guards, atomic saves, atomic commands. |
| Easy | The most common actions are one click or one key away: right-click menus on stage, timeline and layers; a command palette (Ctrl+K) for everything; a shortcut sheet (F1). |
| Smooth flow | Natural, predictable and quick. No modal interruptions for routine edits. A drag is one undo step. Selection, playhead and tool state behave the same everywhere. |

### Power users

* Every command has a stable id, and the important ones have shortcuts.
* **Ctrl+K** opens the command palette (fuzzy search over every command, Enter to run).
* **F1** opens the keyboard-shortcut sheet, generated from the registry so it never goes stale.
* Arrow keys nudge (Shift = 10 px). Tool keys V/R/O/P/N/H/Z. Enter plays.

### Destructive actions (ADR 0004)

Destructive actions are **never a single click or keystroke away from losing work**:

| Tier | Examples | Behaviour |
|---|---|---|
| **Irreversible** (no undo can bring it back) | New/Open/Close with unsaved changes; overwriting a file on disk | Always a confirmation dialog. The destructive button must be clicked (Enter does not confirm). Agents must pass `"confirm": true`. |
| **Undoable, discards notable content** | Delete a layer with artwork; clear a keyframe with artwork; remove a frame that deletes a keyframe | Confirmation dialog by default (Enter confirms, Esc cancels), with "Don't ask again this session". |
| **Routine edit** | Delete selected shapes, move, recolour | No dialog (flow). The status bar says what happened and "Ctrl+Z to undo"; the History panel shows the step. |
