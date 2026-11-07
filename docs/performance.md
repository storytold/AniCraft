# Performance

Rule: **measure first, then optimize, then record the number here.** Every optimization lands
with a benchmark delta and a test that it did not change output (the CPU renderer is the
oracle; its output must stay bit-identical).

## How to measure

```sh
cargo run --release -p anicraft-cli -- bench                      # 50 layers × 200 shapes × 240 frames
cargo run --release -p anicraft-cli -- bench --layers 200 --shapes 500 --frames 600
```

`bench` builds a deterministic stress document (`doc::sample::stress`) and reports file size,
save/load time, evaluate/render time per frame, zoomed render with culling, snapshot cost,
per-edit and per-undo time, history bytes per step and hit-test time. In the app, the status
bar shows render time, shapes drawn and culled, frame-cache size and hit rate, and history
memory.

## Budgets

The default `bench` scenario: 50 layers × 200 shapes × 240 frames, 200k element instances.

| Scenario | Budget | Measured 2026-11-06 (release, 32 threads) | Enforced by |
|---|---|---|---|
| Edit command on a 200k-instance document | < 1 ms | 0.013 ms ✅ | `bench --check` |
| Undo / redo | < 1 ms | 0.009 ms ✅ | `bench --check` |
| Undo memory per small edit | < 16 KB | 2.7 KB ✅ | test, every PR |
| Document snapshot | < 50 µs, shares everything | 0.04 µs ✅ | test (sharing) and `bench --check` |
| Evaluate one frame (200k instances) | < 4 ms | 1.7 ms ✅ | `bench --check` |
| Render 1080p, ~10k visible shapes | < 16.7 ms (60 fps) | **19.5–20.2 ms ❌** | `bench --check` |
| Render at 400% zoom | < 16.7 ms | 11.7 ms ✅ | `bench --check` |
| Shapes drawn at 400% zoom | ≤ 1/8 of the frame's shapes | 5.5% ✅ | test, every PR |
| Hit test | < 200 µs | 52 µs ✅ | `bench --check` |
| File size | < 400 bytes per element instance | 342 ✅ | test, every PR |
| Save / load 65 MB file | < 500 ms | 123 / 146 ms ✅ | `bench --check` |
| UI frame while rendering | never blocked | worker thread ✅ | design (ADR 0002) |

**The 1080p render is over budget.** On the same machine on 2026-11-06, v0.1.0 measures
18.0–18.9 ms and today's `main` 19.4–20.2 ms (most of the difference came with bitmap fills).
The 16.5 ms recorded on 2026-10-28 was not reproduced. Getting back under 16.7 ms is the
next performance task.

### How budgets are enforced

* **Budgets that don't depend on the machine** (undo memory, snapshot sharing, file size,
  culling) are a test (`machine_independent_budgets` in `crates/cli`), so the CI gate fails
  any PR that breaks them.
* **Timing budgets** are `cargo run --release -p anicraft-cli -- bench --check yes`. It
  prints every measurement next to its budget and fails when one is missed. Shared CI
  runners vary too much in speed and core count to gate on time, so this runs on a known
  machine before a release and after performance work. A dedicated benchmark runner would
  let CI run it too.
* The limits live in `TIME_BUDGETS` in `crates/cli/src/lib.rs`; keep that list and this
  table in step.

## Decisions

* [ADR 0001](adr/0001-undo-persistent-snapshots.md): persistent document and snapshot undo
* [ADR 0002](adr/0002-render-pipeline.md): render worker, frame cache, band parallelism, culling
* [ADR 0003](adr/0003-file-format-v2.md): compact file format with shared paths
