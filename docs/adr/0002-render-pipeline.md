# ADR 0002: Render pipeline: background worker, frame cache, band parallelism, culling

* Status: accepted (2026-10-28)

## Context

In M0 the stage rasterized on the UI thread, one shape at a time, with row parallelism only for
big shapes. A synthetic 10k-visible-shape frame took 92 ms at 1080p on 32 threads, and the UI
would freeze for that long on every edit.

## Decision

1. **The UI thread never rasterizes.** `engine::render_service::RenderService` owns a worker
   thread. The stage asks for a frame by key and keeps showing the newest finished picture,
   placed by the stage area it covers so pan and zoom stay smooth, until the new one arrives.
   Policy: one *urgent* slot (latest wins, so a drag never builds a backlog) plus a *prefetch*
   queue, replaced wholesale. During playback the stage prefetches the next 12 frames.
2. **Frame cache keyed by content.** `frame_key(state id, frame, view, onion)` (the onion settings' key since ADR 0047), LRU-bounded by
   bytes (default 768 MB). Undo/redo and looping playback are cache hits.
3. **Two-phase, band-parallel rasterization** (`render::render`):
   * Phase 1, parallel over items: cull by control-point bounds, flatten, stroke and build the
     shader. Display-list order is preserved.
   * Phase 2, parallel over 32-row bands: each band rasterizes every op touching it, in order,
     with one reused coverage buffer. Each row belongs to exactly one band, so output is
     **bit-identical** to a serial render (this keeps the oracle deterministic).
4. **Culling before flattening**, using conservative control-point bounds.
5. **Cheap hit testing**: a bounding-box test rejects items before any curve is flattened. The
   session caches the display list per `(state, frame)`.
6. **Parallel PNG-sequence export** across frames (same bytes as serial export).

## Measured (release, 32 threads, 50 layers × 200 shapes, ≈9.9k visible shapes)

| Metric | Before | After |
|---|---|---|
| 1080p frame | 92.5 ms | 16.5 ms |
| 400% zoom frame (9.2k culled) | 58.1 ms | 10.9 ms |
| Hit test | n/a | 51 µs |

## Consequences / next

* GPU renderer (M3) plugs in behind the same `FrameJob → FrameImage` contract and is checked
  against this CPU oracle.
* Further CPU wins, measured before adopting: a flatten cache keyed by `(Arc<Path>, transform)`
  for static shapes during playback; sparse-strip coverage for very large shapes; SIMD
  compositing.
