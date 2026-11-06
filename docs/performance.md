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

## Budgets (targets)

| Scenario | Budget | Measured 2026-10-28 (release, 32 threads) |
|---|---|---|
| Edit command on a 200k-instance document | < 1 ms | 0.011 ms ✅ |
| Undo / redo | < 1 ms | 0.009 ms ✅ |
| Undo memory per small edit | < 16 KB | 2.7 KB ✅ |
| Document snapshot | < 50 µs | 0.4 µs ✅ |
| Evaluate one frame (200k instances) | < 4 ms | 1.5 ms ✅ |
| Render 1080p, ~10k visible shapes | < 16.7 ms (60 fps) | 16.5 ms ✅ (tight) |
| Render at 400% zoom | < 16.7 ms | 10.9 ms ✅ |
| Hit test | < 200 µs | 51 µs ✅ |
| Save / load 65 MB file | < 500 ms | 118 / 142 ms ✅ |
| UI frame while rendering | never blocked | worker thread ✅ |

Budgets become CI-enforced once the perf harness runs on a fixed machine (roadmap M2).

## Decisions

* [ADR 0001](adr/0001-undo-persistent-snapshots.md): persistent document and snapshot undo
* [ADR 0002](adr/0002-render-pipeline.md): render worker, frame cache, band parallelism, culling
* [ADR 0003](adr/0003-file-format-v2.md): compact file format with shared paths
