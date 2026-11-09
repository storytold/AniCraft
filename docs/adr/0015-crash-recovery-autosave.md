# ADR 0015: Crash-recovery autosave

* Status: accepted (2026-11-09)
* Relates to: roadmap M8 (crash-recovery autosave), ADR 0001 (persistent document), ADR 0003
  (file format), ADR 0004 (destructive actions), ADR 0006 (platform layer)

## Context

A crash, a power cut or a killed process must never cost an artist more than about a minute of
work. Saving the user's own file behind their back is not an option: they decide when their
file changes (Animate's own "auto-save" overwrites the document, and many users turn it off for
that reason). What they need is a separate copy that only matters when something went wrong.

The document is a persistent tree (ADR 0001), so a snapshot is an `Arc` clone (0.05 µs on the
200k-instance stress document), but serializing it is not free (about 125 ms for its 65 MB).
That must never happen on the UI thread.

## Decision

1. **Recovery copies, never the user's file.** Each running editor is one *recovery session*
   with a generated id. Its copy lives in the recovery folder as `<id>.anicraft` (an ordinary
   `.anicraft` file) with a description `<id>.json` (original path, display name, time, app
   version, frames, scenes). The folder is `Recovery` in the per-OS settings folder
   (`platform::recovery_dir`; `ANICRAFT_RECOVERY_DIR` overrides it). Ids are restricted to
   `[A-Za-z0-9_-]`, so a copy can never be written outside the folder, and a write refuses a
   target that is the user's document or a file that isn't a recovery copy.
2. **Atomic writes.** Each file goes to `<name>.tmp`, is flushed to disk (`sync_all`) and renamed
   over the previous one, so a crash mid-write leaves the last good copy.
3. **Liveness by OS file lock.** A running session holds an exclusive lock on `<id>.lock`
   (`std::fs::File::try_lock`, Rust 1.89+; `LockFileEx` on Windows, `flock` elsewhere). The OS
   drops the lock when the process ends, however it ends, so at start-up a copy whose lock can
   be taken was left by a session that didn't close cleanly, and one whose lock is held belongs
   to another open AniCraft window and is not offered. No heartbeat files, no PIDs to guess
   about. Stale locks with no copy are pruned at start.
4. **Background writer.** `recovery::Autosaver` owns a thread. The shell hands it a snapshot
   (O(1)); the thread serializes and writes. Jobs run in order (a clear after a save can never
   overtake the write before it), and when several wait only the newest runs, so a slow disk
   never builds a queue.
5. **When.** A copy is made when the work has been unsaved for the interval (default 1 minute,
   15 s to 10 min) or after N changes (default 25; 0 = by time only), whichever comes first;
   not by change count in the middle of a drag (time still applies). The shell schedules a
   repaint for when the interval runs out, since egui only repaints on input. Preferences ›
   Files › Autosave sets both, and turns it off.
6. **Clean up.** Saving (or undoing back to the saved state, New, Open) clears the session's
   copy. A clean exit (nothing unsaved, or the user chose Don't Save) removes the copy and the
   lock. Any other exit writes the work once more and leaves it for the next start. A panic
   unwinding through `Drop` never deletes anything: only `Autosaver::finish` decides.
7. **Restore / Discard / Later.** At start-up, left-behind copies are listed in a non-blocking
   window (the editor stays usable). **Restore** opens the copy as unsaved work with its
   original path remembered, so Save goes back there, unless that file changed after the
   copy was written (saved just before the crash): then it opens untitled and Save asks for a
   name, so older recovered work never replaces newer work. The restoring session writes its
   own copy at once and then removes the old one, so there is never a moment with no copy.
   **Discard** deletes for good, so it takes a second click on a red button (ADR 0004:
   irreversible; the engine refuses `file.discard_recovery` without `confirm`). **Later**
   keeps everything; File › Recover Autosaved Documents… brings the window back.
8. **Headless.** `file.recovery_list`, `file.recover`, `file.discard_recovery` and
   `file.autosave` take the folder as `dir`, so scripts, the CLI and MCP agents can recover
   work too. A headless `file.autosave` holds no lock (commands are stateless), so its copy is
   offered as left behind; that is the right default for a crashed script.

## Consequences

* One more folder in the settings folder; a crash leaves at most one copy per session there.
* Locking needs a file system with locks. Where `try_lock` is unsupported, liveness can't be
  told and every copy is offered (restoring only reads; the owner rewrites its copy).
* The copy is a full `.anicraft` file each time. On the stress document that is 0.18 s of
  background work per copy and 65 MB written; incremental copies would only matter for much
  larger documents and can come later without changing the folder layout.
* Not yet: several documents open at once (the editor has one), keeping more than the newest
  copy per session, and recovering the undo history (the copy holds the document only).
