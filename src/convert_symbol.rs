//! The Convert to Symbol dialog (F8, ADR 0053): name, type and Library folder, as in Animate.
//! It remembers the folder and type last used, so a run of conversions lands together. What it
//! changes is one `modify.convert_to_symbol`; the choices remembered are view state.

use crate::a11y::Named;
use crate::app::App;
use crate::theme::{FORM_LABEL_DOCK, Type, form_row, text};
use anicraft_engine::doc::{Document, FolderId, LibraryEntry, SymbolKind};
use egui::Key;
use serde_json::json;

/// The dialog while it is open.
pub struct ConvertDialog {
    pub name: String,
    pub kind: SymbolKind,
    pub folder: Option<FolderId>,
    /// The name field takes the keyboard on the first frame.
    focus: bool,
}

/// What the last conversion chose (kept for the next one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Last {
    pub folder: Option<FolderId>,
    pub kind: SymbolKind,
}

/// The folder to offer: the one used last, while it is still in the Library.
pub fn remembered(doc: &Document, last: Last) -> Option<FolderId> {
    last.folder.filter(|f| doc.folder(*f).is_some())
}

/// Opens the dialog for the selection, with the next free "Symbol n" in the remembered folder.
pub fn open(app: &mut App) {
    if app.s.selection.is_empty() {
        // The engine says why ("select something to convert").
        app.request("modify.convert_to_symbol", json!({}));
        return;
    }
    app.playing = false;
    let folder = remembered(&app.s.doc, app.convert_last);
    let name = app.s.doc.numbered_name(folder, "Symbol", app.s.doc.library.len() + 1); // i18n-exempt: a new symbol's default name is document text, as the engine gives it
    app.convert = Some(ConvertDialog { name, kind: app.convert_last.kind, folder, focus: true });
}

fn kind_key(k: SymbolKind) -> &'static str {
    match k {
        SymbolKind::Graphic => "graphic",
        SymbolKind::MovieClip => "movie_clip",
        SymbolKind::Button => "button",
    }
}

/// Where `folder` is, as the dialog names it.
fn folder_label(doc: &Document, folder: Option<FolderId>) -> String {
    match folder {
        Some(f) => doc.entry_path(LibraryEntry::Folder(f)),
        None => tr!("Library (top level)").to_string(),
    }
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    let Some(d) = &mut app.convert else { return };
    let doc = &app.s.doc;
    if d.folder.is_some_and(|f| doc.folder(f).is_none()) {
        d.folder = None;
    }
    let mut folders: Vec<(FolderId, String)> = doc.library_tree.folders.iter().map(|f| (f.id, doc.entry_path(LibraryEntry::Folder(f.id)))).collect();
    folders.sort_by_key(|(_, p)| p.to_lowercase());
    let (mut submit, mut cancel) = (false, false);
    let title = tr!("Convert to Symbol");
    egui::Window::new(title).id(egui::Id::new("convert_symbol")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
        ui.set_min_width(380.0);
        form_row(ui, FORM_LABEL_DOCK, tr!("Name"), |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY)).named(tr!("Symbol name"));
            if d.focus {
                r.request_focus();
                d.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
        });
        form_row(ui, FORM_LABEL_DOCK, tr!("Type"), |ui| {
            egui::ComboBox::from_id_salt("convert_kind")
                .width(200.0)
                .selected_text(crate::panels::symbol_kind_label(d.kind))
                .show_ui(ui, |ui| {
                    for k in [SymbolKind::Graphic, SymbolKind::MovieClip, SymbolKind::Button] {
                        if ui.selectable_label(d.kind == k, crate::panels::symbol_kind_label(k)).clicked() {
                            d.kind = k;
                        }
                    }
                })
                .response
                .set_name(tr!("Symbol type"));
        });
        form_row(ui, FORM_LABEL_DOCK, tr!("Folder"), |ui| {
            egui::ComboBox::from_id_salt("convert_folder")
                .width(200.0)
                .selected_text(folder_label(doc, d.folder))
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                        if ui.selectable_label(d.folder.is_none(), tr!("Library (top level)")).clicked() {
                            d.folder = None;
                        }
                        for (f, path) in &folders {
                            if ui.selectable_label(d.folder == Some(*f), path.as_str()).clicked() {
                                d.folder = Some(*f);
                            }
                        }
                    });
                })
                .response
                .on_hover_text(tr!("The Library folder the new symbol goes into (the next conversion offers it again)"))
                .set_name(tr!("Library folder"));
        });
        // A name the folder already has gets a number, as the engine gives it.
        let wanted = d.name.trim();
        if !wanted.is_empty() && doc.name_taken(d.folder, wanted, None) {
            let becomes = doc.unique_name(d.folder, wanted);
            ui.label(text(Type::Caption, tr!("{folder} already has “{name}”; the new symbol will be “{becomes}”.", folder = folder_label(doc, d.folder), name = wanted, becomes = becomes)));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button(tr!("OK")).clicked() {
                submit = true;
            }
            if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
        });
    });
    if submit {
        accept(app);
    } else if cancel {
        app.convert = None;
    }
}

/// OK: converts with the dialog's choices and remembers the folder and type for next time.
fn accept(app: &mut App) {
    let Some(d) = app.convert.take() else { return };
    let name = d.name.trim();
    let mut params = json!({"kind": kind_key(d.kind), "folder": d.folder.map(|f| f.0)});
    if !name.is_empty() {
        params["name"] = json!(name);
    }
    app.convert_last = Last { folder: d.folder, kind: d.kind };
    if let Some(f) = d.folder {
        app.library.expanded.extend(app.s.doc.folder_chain(f));
    }
    app.request("modify.convert_to_symbol", params);
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::sample;

    #[test]
    fn the_remembered_folder_is_offered_while_it_exists() {
        let mut s = Session::new(sample::demo());
        let f = FolderId(s.execute("library.new_folder", &json!({"name": "Props"})).unwrap()["folder"].as_u64().unwrap());
        let last = Last { folder: Some(f), kind: SymbolKind::MovieClip };
        assert_eq!(remembered(&s.doc, last), Some(f));
        s.execute("library.delete", &json!({"items": [{"folder": f.0}]})).unwrap();
        assert_eq!(remembered(&s.doc, last), None, "a deleted folder falls back to the top level");
        assert_eq!(remembered(&s.doc, Last::default()), None);
    }

    #[test]
    fn ok_converts_into_the_folder_and_the_next_dialog_offers_it_again() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")));
        // Nothing selected: no dialog, the engine says why.
        app.run("edit.deselect", json!({}));
        open(&mut app);
        assert!(app.convert.is_none());
        let f = FolderId(app.run("library.new_folder", json!({"name": "Props"})).unwrap()["folder"].as_u64().unwrap());
        app.run("draw.rect", json!({"x": 10, "y": 10, "w": 20, "h": 20, "object": true}));
        open(&mut app);
        let d = app.convert.as_mut().expect("the dialog opens");
        assert_eq!(d.folder, None, "the top level the first time");
        assert!(d.name.starts_with("Symbol "));
        d.folder = Some(f);
        d.kind = SymbolKind::Button;
        d.name = "  Lamp ".into();
        accept(&mut app);
        assert!(app.convert.is_none());
        let sym = app.s.doc.library.iter().find(|s| s.name == "Lamp").expect("converted, the name trimmed");
        assert_eq!(sym.kind, SymbolKind::Button);
        assert_eq!(app.s.doc.parent_of(LibraryEntry::Item(anicraft_engine::doc::LibraryItem::Symbol(sym.id))), Some(f));
        assert!(app.library.expanded.contains(&f), "the folder opens in the Library");
        // The next conversion offers Props and Button again; an empty name takes the default.
        app.run("draw.rect", json!({"x": 60, "y": 10, "w": 20, "h": 20, "object": true}));
        open(&mut app);
        let d = app.convert.as_mut().expect("the dialog opens");
        assert_eq!((d.folder, d.kind), (Some(f), SymbolKind::Button));
        d.name = String::new();
        accept(&mut app);
        assert_eq!(app.s.doc.library_children(Some(f)).len(), 2);
        // The dialog draws (the clash note too) without the document changing.
        app.run("draw.rect", json!({"x": 90, "y": 10, "w": 20, "h": 20, "object": true}));
        open(&mut app);
        if let Some(d) = app.convert.as_mut() {
            d.name = "Lamp".into();
        }
        let before = app.s.doc.clone();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| window(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert!(app.convert.is_some());
        assert_eq!(app.s.doc, before);
    }
}
