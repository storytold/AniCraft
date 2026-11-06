# ADR 0004: Destructive actions: tiered confirmation, enforced in the engine

* Status: accepted (2026-10-28)

## Context

Requirement from the project lead: destructive actions must always have a confirmation and
must never be a single action away. The users are artists in a flow state, often driving the
app from the keyboard.

## Options considered

| Option | Safety | Flow |
|---|---|---|
| A. Confirm every destructive command (including deleting a shape) | high on paper; in practice users learn to click through constant dialogs ("dialog blindness"), which erodes it | poor: a modal on the most frequent edits |
| B. No confirmations, rely on undo | fine for undoable edits; **fails** for irreversible loss (discarding unsaved work, overwriting files) | best |
| **C. Tiered** (see `product-principles.md`) | irreversible loss always guarded; large undoable deletions guarded by default; routine edits protected by undo with a visible hint | good |

## Decision

**C.** Confirmation rules live in the engine (`engine/src/confirm.rs`, `Session::confirmation`),
not the UI, so every front end gets the same safety:

* The desktop app shows a dialog for both guarded tiers. For irreversible actions the
  destructive button must be clicked; Enter does nothing, Esc cancels.
* Irreversible commands are **refused by the engine** unless the params include
  `"confirm": true`. This protects agents and scripts too.
* Closing the window with unsaved changes asks Save / Don't Save / Cancel.
* Routine deletions report "Ctrl+Z to undo" in the status bar.

## Consequences

* New commands that can discard content must add a rule to `confirm.rs` and a test.
* "Don't ask again" covers only the undoable tier and lasts for the session; it becomes a
  preference when the settings system lands.
