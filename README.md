# AniCraft

An open-source, native 2D vector animation studio in the class of Adobe Animate, written in
Rust. It's a clean-room implementation, part of the `{Function}Craft` family (PhotoCraft,
LightCraft, WordCraft, …).

* **Solid architecture**: layered crates (geom → doc → render → engine → cli → shell). The
  document is plain data, and the picture at a frame is a pure function `evaluate(doc, frame)`.
* **Own renderer**: a deterministic CPU oracle with analytic anti-aliasing (signed-area coverage
  accumulation), non-zero/even-odd fills, gradients, round-join strokes and rayon row parallelism.
* **Headless first**: every action is an engine command. The UI, the CLI and the JSON-lines
  agent channel all dispatch the same ids.
* **egui shell**: stage, tools, timeline (layers, keyframes, classic tweens with easing, onion
  skin), properties and library.

## Build and run

```sh
cargo build                       # whole workspace
cargo run                         # desktop app with the sample animation
cargo run -- --screenshot shots/now.png   # draw ~120 frames, save the window, exit
cargo test                        # all crates
```

## Headless / agents

```sh
cargo run -p anicraft-cli -- info --sample
cargo run -p anicraft-cli -- render --sample --frame 12 --out frame12.png
cargo run -p anicraft-cli -- render-seq --sample --out-dir frames --scale 0.5
cargo run -p anicraft-cli -- commands                 # the registry as JSON
cargo run -p anicraft-cli -- exec --sample draw.rect '{"x":10,"y":10,"w":80,"h":40}' --save out.anicraft
cargo run -p anicraft-cli -- serve --sample           # JSON-lines on stdin/stdout
```

The `serve` protocol takes one request per line,
`{"id":1,"cmd":"timeline.insert_keyframe","params":{"frame":10}}`, and answers
`{"id":1,"ok":true,"result":{…},"revision":7,"frame":0}`. Send `{"cmd":"quit"}` to stop.

## Documents

* [`docs/requirements.md`](docs/requirements.md): scope, functional and non-functional requirements, technology choices
* [`docs/architecture.md`](docs/architecture.md): philosophy, crate map, document model, renderer, engine, testing
* [`docs/roadmap.md`](docs/roadmap.md): honest status and milestones M0–M8
* [`AGENTS.md`](AGENTS.md): rules for contributors and AI agents

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option,
like the sibling app PhotoCraft. Third-party notices are in [`NOTICE`](NOTICE) and
[`ATTRIBUTION.md`](ATTRIBUTION.md). The dependency and asset compliance policy and audit
(`cargo deny`) are in [`docs/legal/compliance.md`](docs/legal/compliance.md). The sample
animation and all UI icons are original and generated in code.

AniCraft is an independent clean-room project. It is not affiliated with or endorsed by Adobe.
Adobe, Animate and Flash are trademarks of Adobe Inc.
