# ADR 0007: Tiled stage rendering with batched, cached tiles

* Status: accepted (2026-10-28)
* Extends: ADR 0002 (render pipeline), ADR 0006 (edit latency)

## Context

Reported: panning reveals a blank stage that fills in shortly after. Cause: the stage rendered
exactly the visible part of the stage as one picture. Any pan exposed pixels that had never been
rendered, and the new picture came one or more frames later. It wasn't CPU speed as such: any
renderer that draws only the viewport shows this.

## Options considered

| Option | Pan | Zoom | Memory | Verdict |
|---|---|---|---|---|
| Render the whole stage every time | never blank | re-render everything | huge at high zoom (a 1080p stage at 800% is 60 MP) | no |
| Overscan the viewport by a margin | blank after a large pan | re-render everything | moderate | half a fix |
| **Tiles on a fixed grid per zoom level, cached** | pans reuse tiles; the margin ring is prefetched | old tiles shown scaled until new ones arrive | bounded by an LRU | **chosen** (the approach used by maps and big-canvas editors) |

## Decision

* The stage is cut into **512 × 512 device-pixel tiles** on a grid anchored at stage (0, 0) for
  the current zoom. Each tile is a `FrameJob` keyed by (state, frame, tile transform, size,
  onion settings; ADR 0047).
* **Batching.** Each UI frame asks for all missing visible tiles in one `request_batch`.
  The worker takes up to 32 jobs, **evaluates the scene once per (state, frame, onion)** and
  renders the tiles in parallel. Newer batches replace pending ones (latest wins). Edits made
  during a frame are already batched: the stage renders once per frame from the final
  document state, never per command.
* **Look-ahead.** The ring of tiles around the view (or, during playback, the next 12 frames'
  visible tiles) goes to the prefetch queue, so normal pans reveal tiles that are already done.
* **No blank stage.** While tiles are missing, tiles from earlier zoom levels or states that
  cover the area are drawn underneath, scaled. Edits still render immediately (ADR 0006), now
  as one parallel batch of the visible tiles.
* **Seamless.** The stage origin is snapped to whole device pixels, so tile edges land on pixel
  boundaries. A test checks that stitched tiles match a single full render to ≤ 1/255 per
  channel, with differences rare: float rounding differs at anti-aliased edges when coordinates
  are offset. Each tile is itself deterministic.
* **Bounded memory.** At most 192 tile textures (about 1 MB each) on the GPU, least recently
  used first out. The render cache is still LRU and byte-bounded (768 MB).
* **Honest timing.** The status bar shows wall-clock time for the view (tiles render in
  parallel), and that number drives Auto mode's immediate-vs-background choice.

## Consequences

* The GPU renderer (M3) keeps the same contract: tiles in, textures out.
* Next: drop tiles more than two zoom levels away early, and use lower-resolution
  "preview" tiles first when zooming far out on huge documents.
