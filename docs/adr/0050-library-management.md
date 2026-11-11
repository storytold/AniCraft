# ADR 0050: Library folders, item management and Swap Symbol

* Status: accepted (2026-11-10)
* Relates to: ADR 0003 (file format), ADR 0004 (destructive actions), ADR 0009 (XFL import),
  ADR 0013 (scripts and linkage), ADR 0019/0032 (GPU stage), ADR 0028 (HTML canvas export),
  ADR 0045 (symbols edited in place), roadmap M2

## Context

The Library listed symbols, bitmaps and sounds in one flat list. It could place an item, delete
a sound and (since ADR 0045) open a symbol for editing. Animate users keep hundreds of items in
nested folders, rename them in place, duplicate symbols to make variants, swap one symbol for
another on the stage, clean out what the movie no longer uses, and set per-item options such as
a bitmap's smoothing. We learned how Animate does this from its public help pages and from
watching the program. Nothing was copied.

## Decision

1. **The organisation is a side table, not a field on each item.**
   `Document.library_tree: Arc<LibraryTree>` (`crates/doc/src/library.rs`) holds the folders
   (`LibraryFolder { id, name, parent }`) and a map from each item (`LibraryItem::Symbol |
   Bitmap | Sound`) to its folder. An item that isn't in the map is at the top level. We chose
   this over a `folder` field on `Symbol`, `Bitmap` and `Sound`:
   * Moving or renaming folders touches one small table, never a symbol (so undo snapshots stay
     O(changed) and symbol timelines keep their `Arc`s).
   * The three item structs, which many places build, are unchanged.
   * Older files have no `library_tree` and open with everything at the top level. A Library
     without folders writes nothing new (`skip_serializing_if`), so files stay the same until
     a folder is made.

   Folder ids come from the document's id counter, like every other id. On load,
   `sanitize_library_tree` repairs a damaged tree: duplicate ids, missing parents, cycles and
   nesting deeper than 32 levels are fixed, and placements of items that aren't there are
   dropped. It touches nothing when the tree is sound. The fuzz checks that no command leaves a
   tree that a load would have to repair.

2. **Names are unique within a folder, ignoring case,** across items and folders, as in Animate.
   They can't be empty, can't be longer than 256 characters, and can't contain `/`, which joins
   a path (`Characters/Hero/Ball`). The engine checks this in `library.new_folder`,
   `library.rename`, `library.move` and `library.duplicate`. A move whose name is already in
   the target folder fails with a message naming it, and nothing moves (commands are atomic).
   We chose refusing over Animate's "replace existing items?" prompt: replacing destroys
   content, and a rename is one keystroke away. Older documents with duplicate names open as
   they are. Convert to Symbol now picks the next free "Symbol n".

3. **Commands** (`crates/engine/src/cmds_library.rs`), all in the existing `library` and
   `element` plug-in namespaces. Entries are named `{"symbol"|"bitmap"|"sound"|"folder": id}`.
   * `library.items`: every folder and item with path, kind, use count and whether it is unused.
   * `library.new_folder`, `library.rename`, `library.move`. A folder can't go inside itself,
     and nesting is limited to 32 levels.
   * `library.duplicate`: a copy right after the original, in the same folder, named
     "<name> copy" (then "copy 2", …). It gets fresh layer and element ids, with layer parents,
     motion tween targets and bone ends following them (`library::fresh_ids`). It has no
     linkage class, because a class names one symbol.
   * `library.delete`: deletes items and folders together with what they hold. What uses them
     goes too, as in Animate: instances of deleted symbols, shapes filled with deleted bitmaps,
     and deleted sounds on keyframes, on every timeline. Only the layers and keyframes that had
     a use are copied. Deleting the symbol that is open for editing closes it (ADR 0045).
   * `library.unused` (Select Unused Items) and `library.delete_unused`.
   * `library.set_smoothing`; `element.swap_symbol`. Linkage stays `script.set_class`
     (ADR 0013).

4. **Use count and unused have different meanings, on purpose.** Use count is the number of
   references on every timeline (scenes and symbols), counted once per keyframe: instances,
   bitmap fills and keyframe sounds. *Unused* means **not reachable from any scene**. A symbol
   used only inside a symbol that nothing places is unused, even though its count is above
   zero. One Delete Unused Items then clears a whole dead branch in one undo step, instead of
   peeling off one layer per run. Symbols with a linkage class count as reached, because scripts
   create them by name (ADR 0013). The row's tooltip and screen-reader name give both.

5. **Confirmations follow ADR 0004** (undoable tier, `crates/engine/src/confirm.rs`). Deleting
   asks only when something deleted is in use ("used 4 times in the movie; those uses go too")
   or when a folder holds items ("holds 5 items, deleted with it"). Deleting an unused item or
   an empty folder is routine and doesn't ask. Delete Unused Items always asks and names the
   first few items.

6. **Swap Symbol** replaces only the symbol an instance shows. Transform, colour effect,
   blending, filters, looping, actions and instance name stay, as in Animate. It is a button in
   Properties › Symbol ("Swap…", listing symbols by path). Inside a symbol open for editing, it
   refuses a symbol that would contain itself (`editing::would_nest`).

7. **Allow Smoothing** is a property of the bitmap (`Bitmap.smoothing`, on by default; only
   `false` is written). It reaches the renderers through `DisplayItem.smooth`. The CPU oracle
   samples the nearest pixel instead of bilinear (`Shader::smoothed`). The GPU stage uses bit 1
   of the bitmap's flag word in `fine.wgsl` and does the same, so oracle parity still holds.
   SVG export marks the image `image-rendering="pixelated"`, and the HTML page turns
   `imageSmoothingEnabled` off for that fill. XFL import reads `allowSmoothing`.

8. **XFL import keeps folders.** `<folders><DOMFolderItem name="A/B"/>` creates folders, empty
   ones included. Symbols and bitmaps go into the folder their library name's path names
   (`Characters/Ball` → `Ball` in `Characters`). The report lists "library folders" as kept.

9. **The panel (`src/library.rs`) holds only view state**: selection (Ctrl-click and
   Shift-click ranges), open folders, sort (Name, Type, Use count, Date added, ascending or
   descending; folders first), the search text, and the item being renamed. Everything that
   changes the document is a command.
   * "Date added" is the item's id order: ids only grow, so no clock is stored.
   * Searching opens every folder that leads to a match.
   * F2, or a double-click on the name, renames in place. Enter or clicking away keeps the new
     name; Esc cancels. A double-click on the icon edits a symbol (ADR 0045) or places an image
     or sound.
   * Rows drag onto a folder (into it), onto an item (into that item's folder) or below the
     list (to the top level).
   * F2 and Delete act on the Library only while the list was the last thing clicked, so
     Delete on the stage still clears artwork.
   * The preview shows a symbol's first frame rendered by the CPU oracle, a bitmap drawn with or
     without smoothing as on the stage, or a sound's waveform with a play button. Below it are
     the item's properties: a symbol's type and linkage class, a bitmap's Allow Smoothing, a
     sound's format.
   * Use counts are worked out once per document state.

## Consequences

* Measured on 200,000 element keys plus 1,000 symbols in 50 folders, release build: use counts
  1.5 ms, unused items 1.7 ms, the whole `library.items` listing 5.0 ms (`library_tests.rs`).
* Duplicates share no ids with their originals. Bone ids inside an armature are kept: they are
  scoped to their layer.
* One XFL corpus golden changed: its checker bitmap says `allowSmoothing="false"` and is now
  drawn with hard edges, as Animate shows it.

## Not yet

(Dragging items onto the stage, choosing a folder in Convert to Symbol and Swap Bitmap came in
ADR 0053.) Animate's Properties dialog (the properties are inline),
linkage on bitmaps and sounds, bitmap compression settings, Update and Import of library items
from another document (shared libraries), folder expansion saved in the file and sorting by size.
