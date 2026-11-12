use super::*;
use crate::{Element, Layer, Linkage, Symbol, SymbolKind, sample};

fn instance(d: &mut Document, symbol: SymbolId) -> Arc<Element> {
    Arc::new(Element {
        id: ElementId(d.alloc_id()),
        name: String::new(),
        transform: Default::default(),
        color: Default::default(),
        kind: ElementKind::Symbol { symbol, loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
    })
}

/// A symbol named `name` holding instances of `inside`.
fn add_symbol(d: &mut Document, name: &str, inside: &[SymbolId]) -> SymbolId {
    let id = SymbolId(d.alloc_id());
    let mut layer = Layer::new(LayerId(d.alloc_id()), "Layer 1");
    let els: Vec<_> = inside.iter().map(|s| instance(d, *s)).collect();
    if let Some(k) = layer.keyframe_mut(0) {
        k.elements = els;
    }
    d.library.push(Arc::new(Symbol { id, name: name.into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(layer)] } }));
    id
}

fn sym(d: &Document, name: &str) -> SymbolId {
    d.library.iter().find(|s| s.name == name).map(|s| s.id).unwrap_or(SymbolId(0))
}

#[test]
fn folders_round_trip_and_older_files_open_without_them() {
    let mut d = sample::demo();
    let plain = serde_json::to_string(&d).unwrap();
    assert!(!plain.contains("library_tree"), "a Library without folders writes nothing new");
    let back: Document = serde_json::from_str(&plain).unwrap();
    assert!(back.library_tree.is_empty());

    let chars = d.ensure_folder_path("Characters/Hero").unwrap();
    let ball = sym(&d, "Ball");
    d.place_entry(LibraryEntry::Item(LibraryItem::Symbol(ball)), Some(chars));
    let json = serde_json::to_string(&d).unwrap();
    assert!(json.contains(&format!("{{\"symbol\":{},\"folder\":{}}}", ball.0, chars.0)), "{json}");
    let mut back: Document = serde_json::from_str(&json).unwrap();
    back.sanitize();
    assert_eq!(back.library_tree, d.library_tree);
    assert_eq!(back.entry_path(LibraryEntry::Item(LibraryItem::Symbol(ball))), "Characters/Hero/Ball");
}

#[test]
fn folder_paths_are_made_once_and_matched_without_case() {
    let mut d = Document::default();
    let a = d.ensure_folder_path("Characters/Hero").unwrap();
    let b = d.ensure_folder_path(" characters / HERO ").unwrap();
    assert_eq!(a, b);
    assert_eq!(d.library_tree.folders.len(), 2);
    assert_eq!(d.folder_depth(a), 2);
    assert_eq!(d.ensure_folder_path(""), None);
    assert_eq!(d.ensure_folder_path("//"), None);
    let top = d.folder_chain(a)[0];
    assert_eq!(d.folder_height(top), 2);
    assert!(d.folder_within(a, top) && !d.folder_within(top, a));
    // Nesting stops at the limit instead of growing without end.
    let deep = vec!["x"; MAX_FOLDER_DEPTH + 10].join("/");
    let f = d.ensure_folder_path(&deep).unwrap();
    assert_eq!(d.folder_depth(f), MAX_FOLDER_DEPTH);
}

#[test]
fn names_are_unique_per_folder_without_case() {
    let mut d = sample::demo();
    let f = d.ensure_folder_path("Props").unwrap();
    let ball = LibraryEntry::Item(LibraryItem::Symbol(sym(&d, "Ball")));
    assert_eq!(d.check_name(None, "ball", None), Err(NameError::Taken("ball".into())));
    assert_eq!(d.check_name(None, " Ball ", Some(ball)).as_deref(), Ok("Ball"), "an item may keep its own name");
    assert_eq!(d.check_name(Some(f), "Ball", None).as_deref(), Ok("Ball"), "another folder may hold the same name");
    assert_eq!(d.check_name(None, "a/b", None), Err(NameError::Slash));
    assert_eq!(d.check_name(None, "  ", None), Err(NameError::Empty));
    assert_eq!(d.check_name(None, &"x".repeat(MAX_NAME_CHARS + 1), None), Err(NameError::TooLong));
    assert_eq!(d.check_name(None, "props", None), Err(NameError::Taken("props".into())), "folders and items share a folder's names");
    assert_eq!(d.unique_name(None, "Ball"), "Ball 2");
    assert_eq!(d.unique_name(None, "Ball copy"), "Ball copy");
    assert_eq!(d.numbered_name(None, "Symbol", 1), "Symbol 1");
    add_symbol(&mut d, "Symbol 1", &[]);
    assert_eq!(d.numbered_name(None, "Symbol", 1), "Symbol 2");
}

#[test]
fn damaged_trees_are_repaired_on_load() {
    let mut d = sample::demo();
    let ball = LibraryItem::Symbol(sym(&d, "Ball"));
    let t = Arc::make_mut(&mut d.library_tree);
    let f = |id: u64, name: &str, parent: Option<u64>| LibraryFolder { id: FolderId(id), name: name.into(), parent: parent.map(FolderId) };
    t.folders = vec![f(900, "A", Some(901)), f(901, "B", Some(900)), f(902, " ", Some(902)), f(903, "C/D", Some(999)), f(900, "Dup", None)];
    t.placed.insert(ball, FolderId(901));
    t.placed.insert(LibraryItem::Sound(SoundId(12345)), FolderId(900));
    t.placed.insert(LibraryItem::Symbol(SymbolId(77777)), FolderId(900));
    d.sanitize();
    let t = &d.library_tree;
    assert_eq!(t.folders.len(), 4, "the duplicate id is dropped");
    assert!(t.folders.iter().all(|f| d.folder_chain(f.id).len() <= 2), "the cycle is broken");
    assert!(t.folders.iter().all(|f| f.parent != Some(f.id) && f.parent.is_none_or(|p| d.folder(p).is_some())));
    assert!(t.folders.iter().all(|f| clean_name(&f.name).is_ok()), "{:?}", t.folders);
    assert_eq!(t.placed.len(), 1, "only items that exist stay placed");
    assert!(d.next_id > 903, "folder ids stay unique");
    // A sane tree is left alone (same Arc).
    let before = d.library_tree.clone();
    d.sanitize();
    assert!(Arc::ptr_eq(&before, &d.library_tree));
}

#[test]
fn uses_count_every_keyframe_and_unused_follows_reachability() {
    let mut d = sample::demo();
    let (ball, star) = (sym(&d, "Ball"), sym(&d, "Star"));
    let uses = d.library_uses();
    assert!(uses.get(&LibraryItem::Symbol(ball)).copied().unwrap_or(0) > 0);
    assert!(uses.get(&LibraryItem::Symbol(star)).copied().unwrap_or(0) > 0);
    assert!(d.unused_items().is_empty(), "the sample uses everything");

    // Inner is used only inside Outer, which nothing places: both are unused, though Inner
    // has a use.
    let inner = add_symbol(&mut d, "Inner", &[]);
    let outer = add_symbol(&mut d, "Outer", &[inner, inner]);
    assert_eq!(d.library_uses().get(&LibraryItem::Symbol(inner)), Some(&2));
    assert_eq!(d.unused_items(), vec![LibraryItem::Symbol(inner), LibraryItem::Symbol(outer)]);
    // A linkage class makes a symbol reachable (scripts make it by name), and what it holds.
    if let Some(s) = d.library.iter_mut().find(|s| s.id == outer) {
        Arc::make_mut(s).linkage = Some(Linkage { class: "Outer".into(), base: String::new() });
    }
    assert!(d.unused_items().is_empty());
    // Cycles terminate.
    let a = add_symbol(&mut d, "A", &[]);
    let b = add_symbol(&mut d, "B", &[a]);
    if let Some(s) = d.library.iter_mut().find(|s| s.id == a) {
        let layer = Arc::make_mut(&mut Arc::make_mut(s).timeline.layers[0]);
        let e = Arc::new(Element { id: ElementId(5_000_000), name: String::new(), transform: Default::default(), color: Default::default(), kind: ElementKind::Symbol { symbol: b, loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() } });
        layer.keyframe_mut(0).unwrap().elements.push(e);
    }
    assert_eq!(d.unused_items(), vec![LibraryItem::Symbol(a), LibraryItem::Symbol(b)]);
    assert!(d.symbol_contains(a, b) && d.symbol_contains(b, a) && d.symbol_contains(a, a));
    assert!(!d.symbol_contains(inner, outer) && d.symbol_contains(outer, inner));
}

#[test]
fn a_duplicate_has_fresh_ids_and_keeps_its_folder() {
    let mut d = sample::demo();
    let star = sym(&d, "Star");
    let f = d.ensure_folder_path("FX").unwrap();
    d.place_entry(LibraryEntry::Item(LibraryItem::Symbol(star)), Some(f));
    if let Some(s) = d.library.iter_mut().find(|s| s.id == star) {
        Arc::make_mut(s).linkage = Some(Linkage { class: "Star".into(), base: String::new() });
    }
    let copy = d.duplicate_symbol(star, "Star copy".into()).unwrap();
    let (a, b) = (d.symbol(star).unwrap(), d.symbol(copy).unwrap());
    assert_eq!(b.name, "Star copy");
    assert_eq!(b.kind, a.kind);
    assert!(b.linkage.is_none(), "a class names one symbol");
    assert_eq!(d.parent_of(LibraryEntry::Item(LibraryItem::Symbol(copy))), Some(f));
    let ids = |s: &Symbol| -> HashSet<u64> { s.timeline.layers.iter().flat_map(|l| std::iter::once(l.id.0).chain(l.keyframes.iter().flat_map(|k| k.elements.iter().map(|e| e.id.0)))).collect() };
    assert!(ids(a).is_disjoint(&ids(b)));
    assert_eq!(ids(a).len(), ids(b).len());
    // Same artwork: only ids differ, and an element keeps one id across its keyframes.
    for (la, lb) in a.timeline.layers.iter().zip(&b.timeline.layers) {
        assert_eq!(la.keyframes.len(), lb.keyframes.len());
        for (ka, kb) in la.keyframes.iter().zip(&lb.keyframes) {
            for (ea, eb) in ka.elements.iter().zip(&kb.elements) {
                assert_eq!((&ea.kind, ea.transform), (&eb.kind, eb.transform));
            }
        }
        let first: Vec<u64> = lb.keyframes.iter().filter_map(|k| k.elements.first().map(|e| e.id.0)).collect();
        if first.len() > 1 && la.keyframes.iter().filter_map(|k| k.elements.first().map(|e| e.id.0)).collect::<HashSet<_>>().len() == 1 {
            assert!(first.windows(2).all(|w| w[0] == w[1]));
        }
    }
    let order: Vec<SymbolId> = d.library.iter().map(|s| s.id).collect();
    let at = order.iter().position(|s| *s == star).unwrap();
    assert_eq!(order[at + 1], copy, "the copy sits right after its original");
    assert_eq!(d.duplicate_symbol(SymbolId(424242), "x".into()), None);
}

#[test]
fn folder_contents_reach_every_depth() {
    let mut d = sample::demo();
    let a = d.ensure_folder_path("A").unwrap();
    let ab = d.ensure_folder_path("A/B").unwrap();
    let c = d.ensure_folder_path("C").unwrap();
    let ball = LibraryItem::Symbol(sym(&d, "Ball"));
    d.place_entry(LibraryEntry::Item(ball), Some(ab));
    let inside = d.folder_contents(a);
    assert_eq!(inside, vec![LibraryEntry::Folder(ab), LibraryEntry::Item(ball)]);
    assert!(d.folder_contents(c).is_empty());
    assert_eq!(d.library_children(Some(ab)), vec![LibraryEntry::Item(ball)]);
    assert!(d.library_children(None).starts_with(&[LibraryEntry::Folder(a), LibraryEntry::Folder(c)]));
    assert!(!d.library_children(None).contains(&LibraryEntry::Item(ball)));
}
