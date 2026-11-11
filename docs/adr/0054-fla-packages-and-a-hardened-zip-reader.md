# ADR 0054: `.fla` packages: corpus variants and a hardened zip reader

* Status: accepted (2026-11-11)
* Relates to: ADR 0009 (XFL import, own zip reader), ADR 0035 (the XFL corpus and golden
  renders), ADR 0036 (web build: a `.fla` handed over as bytes), roadmap M4

## Context

Artists open `.fla` files: a zip archive of the same XFL tree the corpus (ADR 0035) tests as
folders. The zip reader from ADR 0009 read stored and deflate entries and capped entry sizes,
but nothing tested it against the layouts real zip writers produce (folder entries, `\`
separators, unflagged or CP437 names, data descriptors, ZIP64), and nothing tested it against
an archive made to hurt: entries sharing bytes (the overlapping-file zip bomb), sizes and
checksums that lie, names with `..`, absolute or drive paths, the same name twice, very deep
folders, a central directory cut short. A `.fla` arrives from anywhere (mail, the web build's
file picker), so it is hostile input.

## Decision

1. **Read what writers write.** The reader (`crates/xfl/src/zip.rs`, from PKWARE's APPNOTE
   6.3) takes stored and deflate entries; folder entries; `\` read as `/`; empty and `.` path
   parts dropped; data descriptors (sizes come from the central directory, which always has
   them); **ZIP64** end records and per-entry sizes (cheap to read, written by some tools for
   small files too, and refused archives would be a worse answer); names as UTF-8 when flag
   bit 11 or the Info-ZIP Unicode path field (0x7075, checked against the name's CRC) says so,
   else **CP437** as the format specifies, except that a name that is valid UTF-8 is read as
   UTF-8 (many writers store UTF-8 without the flag; ASCII reads the same either way, and CP437
   text that happens to be valid UTF-8 is vanishingly rare). A name flagged UTF-8 that isn't is
   an error, not a guess.
2. **Refuse, with a sentence, what no honest writer makes.** Opening checks the whole archive
   before any entry is inflated: every offset and length is checked against the bytes it
   comes from; each entry's local header and data must end before the central directory, and
   **no two entries may share bytes** (sorted spans); a path that would leave the package is
   refused (`..`, leading `/` or `\`, a drive in the first part, NUL), as are **duplicate
   paths** (after `\` → `/`), paths inside more than **32 folders**, more than **100,000
   entries**, and split archives. Reading an entry refuses encryption and other methods by
   name ("LZMA compression"), and checks the result: an entry must inflate to exactly its
   stated size (inflation stops at that size) and match its **CRC-32**.
3. **Bombs: a ratio cap, an entry cap and a total budget.** Deflate can't pass about 1032 : 1,
   so an entry claiming more than **1024 : 1** (plus 4 KB) is refused before inflating as a
   bomb or a damaged size; no entry inflates past **256 MB**; and one archive hands out at
   most **1 GiB** over every read, so a document that names one large entry many times can't
   multiply it. The `.fla` itself is capped at 1 GiB, as before.
4. **Nothing is extracted.** The reader only answers lookups from the importer, in memory; it
   never writes a file, so path checks keep lookups unambiguous and match the folder reader,
   which also refuses `..`, empty parts and drives. Safe Rust slicing means no read leaves the
   archive's bytes.
5. **Every file form opens the same.** `import_path` takes an XFL folder, the `.xfl` pointer
   file inside it (or any file there, such as `DOMDocument.xml`), or a `.fla`, recognised by
   its first bytes too, so a renamed archive opens. A `.fla` saved by Flash CS4 or earlier (a
   compound binary file) is named in the error, with what to do: open it in Animate and save
   it again. `import_fla_with_fonts` joins `import_with_fonts` for tests and hosts.
6. **The corpus in every layout, by an independent writer.** `crates/xfl/tests/support/fla.rs`
   packs a package into `.fla` archives in memory, written from the specification separately
   from the reader (its own CRC), in nine variants: as Animate writes it (the stored
   `mimetype` entry first), stored, mixed (XML deflated, images stored) with folder entries,
   Windows separators with unflagged names, CP437 names, Unicode path fields, data
   descriptors, ZIP64, and all of these at once. Every package must import **identically**
   to its folder (the same `Document`, the same report), from bytes and from disk, and render
   the same golden frames (`crates/engine/tests/xfl_corpus.rs`); one package is also checked
   with library names outside ASCII in every encoding.
7. **A fuzz built from the corpus.** `hostile_fla_archives_never_panic_and_fail_readably`
   sets every central header field of the first entries to edge values, changes random bytes
   (in the directory, then anywhere), cuts each archive short, and builds archives with unsafe
   names, duplicates, overlaps, lying sizes and checksums, ratio bombs and deep folders:
   9,664 archives per run, none may panic, every failure must be a readable sentence, and the
   built attacks must all be refused.
8. **CRC-32 by slicing by 8** (eight table lookups per eight bytes, Kounavis and Berry, 2005):
   checking every entry costs little. On a generated 54.6 MB `.fla` (400 symbols, 24 stored
   PNGs, 65.8 MB inflated), reading every entry fell from 128–140 ms (byte-wise table) to
   53–54 ms, and the whole import from 388–458 ms to 311–328 ms (`docs/performance.md`).

## Consequences

* A damaged or hostile `.fla` fails at once with a sentence that names the entry and the
  problem, and never takes the app down or its memory.
* A good archive that some writer lays out in a way we haven't seen fails loudly rather than
  importing wrongly; a real file that does should become a new variant in the packer, not a
  special case.
* Not in this slice: archives with data before the first entry (self-extracting zips, whose
  offsets are shifted), an XFL folder zipped inside a top folder (`name/DOMDocument.xml`),
  encrypted archives and other compression methods (all refused with a clear error), and
  sound and video items (reported, as before).
