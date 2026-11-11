# ADR 0035: The XFL corpus and golden render tests

* Status: accepted (2026-11-09)
* Relates to: ADR 0009 (XFL import), ADR 0014 (XFL text), ADR 0023 (shape hints), ADR 0002
  (render pipeline: the CPU renderer is the oracle), roadmap M4

## Context

XFL import had grown feature by feature (shapes, symbols, bitmaps, masks, folders, scenes,
text, filters, blend modes, shape tweens and hints), each tested with a small inline snippet
that checked the model. Nothing checked what an imported file *looks like*, nothing exercised
the features together in files shaped like Animate's (namespaces, declarations, comments,
library folders, `.xfl` proxy files, attributes we ignore), and the hostile-input test was one
hand-written document. A regression in edge chaining, a gradient matrix or a mask link would
pass every test as long as the model kept its shape.

We may not use Animate's own files (clean room, ADR 0009), so the corpus has to be written by
hand from the public descriptions of the format.

## Decision

1. **A corpus of self-authored packages** in `crates/xfl/tests/corpus/<name>/`, each a real
   XFL folder: `DOMDocument.xml`, `LIBRARY/` (symbols and images), and `<name>.xfl`. Eleven
   packages, one per area, together covering every feature the importer claims: `shapes`
   (every edge command, both selectors of lines and curves, `S` marks, hex fixed point, a fill
   on both sides of an edge, a hole, alpha, every cap and join, strokes continuing across
   edges and with gaps), `primitives` (rectangles, ovals, nested groups, a hidden layer),
   `gradients` (linear and radial with matrices, rotation, repeat and reflect, a focal point, an
   ellipse, linear-RGB, a gradient stroke), `bitmaps` (PNG and JPEG instances, repeating and
   clipped fills, a missing image), `symbols` (graphics in every loop mode, nesting, a movie
   clip, a button, every colour effect, a missing symbol), `tweens` (classic with eases, a
   custom ease, extra spins, a shape tween with a hint, a motion tween), `masks` (top level, in
   a folder, inside a symbol), `guides` (plain and motion guides), `text` (static, dynamic and
   input fields, several runs), `filters` (every filter, a stack, switched off, unknown, and
   blend modes) and `scenes` (three scenes, labels, a comment, an anchor, frame scripts). Every
   image is original (made for the corpus, listed in `ATTRIBUTION.md`).
2. **Three kinds of check per package.**
   * *Report and structure* (`crates/xfl/tests/corpus.rs`): the exact report each package
     must produce (every line is something it uses on purpose), and targeted checks of the
     model (loops per fill, loop modes, eases, mask and guide links, labels, scenes).
   * *Hostile variants* (same file): every file of every package cut short at 26 points,
     garbled 96 ways (deterministic xorshift: overwritten bytes, inserted markup and edge
     commands and out-of-range numbers, deleted and repeated spans, changed digits), and every
     file removed in turn. Import may fail, never panic, and what it imports must evaluate.
     About 3,000 variants, run in a quarter of a second.
   * *Golden renders* (`crates/engine/tests/xfl_corpus.rs`): chosen frames of every package
     rendered by the CPU oracle at half size (160 × 90) and compared with committed PNGs in
     the package's `golden/` folder, plus probes that check hand-worked colours at chosen
     points, so a golden that drifted from what the file describes can't be accepted quietly.
     Each package also opens with `file.open` and round-trips through `.anicraft`.
3. **Goldens live in the engine's tests, the corpus in the xfl crate.** The renderer and the
   importer are both L2, and a crate may not depend on its own layer (dev-dependencies
   included, `src/architecture_tests.rs`). The engine (L3) already depends on both and is
   where `file.open` imports, so its integration tests render the corpus in place. We chose
   this over relaxing the layering rule for dev-dependencies (a rule with exceptions stops
   being checked) or moving rendering checks into the CLI (L4, further from the importer).
4. **The golden helper** (`crates/engine/tests/golden/mod.rs`, reusable by any future corpus):
   * compares straight-alpha RGBA8; a pixel differs when a channel is off by more than 3, and
     the check fails when more than 0.2 % of pixels differ. That absorbs floating-point
     differences on anti-aliased edges between computers, and catches a shape moved by a
     pixel or a colour changed anywhere it covers;
   * `UPDATE_GOLDENS=1` writes the renders as the goldens; a missing golden fails (CI never
     makes its own) and says how to make it;
   * on a mismatch it writes the render and a diff image (the golden faded, differing pixels
     in red) under Cargo's integration-test temporary folder and names both paths.
5. **Text renders without fonts.** Text is laid out with the fonts on the computer, which no
   two computers share and which we may not commit. The golden renders import with no fonts
   (text reported, the rest drawn), so they match everywhere; the text package's blocks are
   checked structurally with whatever fonts the computer has.
6. **Importer fixes the corpus found** (each covered by the corpus and unit tests):
   * a library symbol without `symbolType` is a **movie clip** (the format leaves defaults out;
     only graphics and buttons say what they are). It was imported as a graphic;
   * **brightness** colour effects were dropped; they are now a tint towards white or black by
     that amount (the same colour change). Advanced colour effects keep their alpha and are
     reported;
   * **instance names** of movie clips and buttons were dropped, so actions couldn't address
     imported instances (ADR 0024); names that are identifiers are now kept, others reported;
   * a **frame comment** (`labelType="comment"`) was imported as a label that actions could
     go to; it is now left out and reported (anchors stay labels);
   * a **custom ease** of one segment is now the cubic Bézier it is; longer curves and
     per-property curves keep the slider's ease and are reported;
   * reverse loop modes play forward and are reported;
   * what AniCraft draws the nearest way is now reported instead of silently changed: caps and
     joins other than round (drawn as they are since ADR 0046), gradient and bitmap strokes, repeating and reflecting gradients,
     focal points, elliptical radial gradients, linear-RGB gradients, and extra tween spins.

## Consequences

* An importer or renderer change that alters how a corpus file looks fails with the paths of
  the render and the diff; an intended change is accepted by rerunning with
  `UPDATE_GOLDENS=1` and looking at the new PNGs (they are small: 20 frames, about 120 KB).
* The corpus is the place to add a feature's file when the importer learns it. Two tests
  refuse a package without report expectations or golden frames.
* The corpus is written from the format's public description, not from Animate's output, so
  it can share our misreadings of the format. Where we were unsure (the default symbol type,
  the custom ease and shape-hint forms) this ADR says so, and a file saved by Animate that
  disagrees should become a corpus fix, not a special case.
* `.fla` (zip) variants of the corpus are checked since ADR 0054: every package, packed nine
  ways, imports identically to its folder and renders the same goldens.
* Not in this slice: sound and video items, motion tweens
  with property curves, and per-character text styles (all reported, and kept for their own
  slices).
