# ADR 0101: Moho import (.mohoproj/.moho) and the maop_maohou sidecar

* Status: accepted (2026-11-09)
* Relates to: ADR 0003 (file format), ADR 0009 (XFL import), ADR 0012 (text), the Moho
  parity plan (fork plan › stage 1), maop-corpus (test corpus)

## Context

The fork's goal is 1:1 Moho Pro 14 capability parity, starting with import: a Moho document
(`.mohoproj` bare JSON, `.moho` zip package) must open in AniCraft, and AniCraft's native
`.anicraft` format must keep whatever the document model cannot hold yet, so later milestones
(bones, channels, effects) can re-project it without the original file. All format knowledge
here is clean-room: the corpus fixtures we generated ourselves (maop-corpus, Moho Pro 14.4
native saves of our own drawings), the vendor's public manual and scripting docs, and
black-box compat-gate tests of files we authored.

## Decision

1. **A new L2 importer crate, `crates/mohoproj-import`** (the XFL pattern: own zip reader,
   lenient bounded JSON access, "report, don't guess"). `import_path` takes a `.moho`
   package, a bare `.mohoproj`, or an extracted folder; `file.open` dispatches on it exactly
   as on Animate files, and the import report rides the command result.
2. **Static full import.** Stage, frame rate and background from `project_data`; the inline
   layer tree flattened (Moho lists layers bottom-first — corpus-derived: CharWalk lists a
   rig's right limbs before Body/Head and pupils after eye whites — while AniCraft's index 0
   is the top, so the walk visits siblings back-to-front and emits a container before the
   children it flattens, keeping every folder's children in one unbroken run); Group/Bone/
   Switch containers flatten to folders (a switch freezes at its selection; unselected
   variants hide); meshes become paths with solid, linear and radial gradient fills and solid
   strokes; text layers lay out with this computer's fonts; image layers load their file from
   the package or from beside the document. Coordinates stay in Moho inches (y up, centre
   origin) in path data; the element transform carries them to the stage at Moho's project
   scale of 320 px/inch. Channel values are sampled at the document's start frame; channels
   with more than one key count as animation not yet imported.
3. **Curve handles are reconstructed, and the reconstruction is reported.** Moho derives
   Bézier handles per point from `smoothness`, `weight_in/out` and `offset_in/out`; the exact
   formula is not publicly documented. The importer uses tangent-direction handles of length
   `distance/3 · smoothness · weight`, rotated by `offset`, zero at smoothness 0 — an
   approximation good enough to import structure, checked against Moho renders in the later
   render-comparison milestones. The untouched mesh rides in the sidecar meanwhile.
4. **The `maop_maohou` sidecar lives on the document** (`Document.maop_maohou:
   Option<Arc<MaohouSidecar>>` with `schema`, `saved_by`, `source` and `raw` = the source
   file's whole JSON). This is the `.maop` dialect's extension node under the fork's dual-write
   rule: the mapped document stays standard, Moho-only data rides beside it. Placement on the
   Document (not the envelope) keeps it correct through save/load, undo snapshots and
   sanitisation with no engine plumbing, and Arc sharing keeps cloning O(1). `FORMAT_VERSION`
   moves 3 → 4 to mark the possible section; v3 files load unchanged (the field defaults to
   `None`), and the sidecar round-trips byte-structure-equal.
5. **Units decisions, all in one place** (`MOHO_DPI`, text size, justification mapping) with
   their corpus evidence, to be re-validated against Moho renders by the later
   render-comparison gates; corrections then happen in one module.

## Consequences

* Moho documents open with everything static drawn; the report names what is not yet
  supported (bones, keyframed channels, layer effect stacks, reflected/angle gradients,
  physics, camera animation, …), and nothing falls out of the file silently.
* `.anicraft` v4 files are bigger by the sidecar; native documents are unchanged.
* The corpus gate (`fixtures` tests in the crate, plus the engine round-trip test) needs the
  maop-corpus next to the workspace or at `MAOHOU_FIXTURES`; without it the suite still
  passes, printing why it skipped.
* The compat-gate items that need a running Moho 14.4 (opening files written for Moho)
  remain owner-side real-machine checks; the recorded 2026-11-05/07 runs stand as evidence
  for the dialect's read tolerance.
