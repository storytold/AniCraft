//! The editor in a web browser (ADR 0036, docs/web.md): the start on the page's canvas, and
//! what the browser offers in place of the operating system's services. `platform.rs` decides
//! when they are used; this module only talks to the browser.
//!
//! * **Files.** The browser's file picker brings a file in; its bytes go to the engine's
//!   in-memory store (`engine::vfs`) and the command runs on it. What a save or an export
//!   writes there leaves as a download.
//! * **Settings and crash-recovery copies** (ADR 0015) live in the browser's local storage for
//!   this site. Each tab holds a Web Lock named after its session for as long as it is open,
//!   so the copies offered at start are those of tabs that closed or crashed, never those of a
//!   tab still open (the same rule as the desktop's lock files).
//! * **Leaving the page** with unsaved work asks first, as closing the window does.
//!
//! Nothing here can panic on what the browser returns: every call that can fail is checked
//! and a failure degrades (no storage: autosave is off and says so; no picker: nothing opens).

use crate::platform::Picked;
use anicraft_engine::doc::Document;
use anicraft_engine::recovery::{Entry, Outcome, RecoveryMeta, WriteStats};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Where the preferences are kept.
pub const PREFS_KEY: &str = "anicraft.prefs";
/// Recovery copies: `<prefix><session>.doc` (the document) and `.meta` (its description).
const RECOVERY_PREFIX: &str = "anicraft.recovery.";
/// Each open tab's Web Lock: this prefix and its session id.
const LOCK_PREFIX: &str = "anicraft-session-";
/// The page's canvas and loading message (packaging/web/index.html).
const CANVAS_ID: &str = "anicraft_canvas";
const LOADING_ID: &str = "anicraft_loading";
const PICKER_ID: &str = "anicraft_file_input";
/// Largest file taken from the picker (the engine's own limits are lower for most kinds).
const MAX_PICK_BYTES: f64 = 512.0 * 1024.0 * 1024.0;

/// Files read from a drop: each one's name, and its bytes or why the browser couldn't read it.
pub type ReadFiles = Vec<(String, Result<Vec<u8>, String>)>;

thread_local! {
    /// Files the picker delivered, waiting for the next frame.
    static PICKED: RefCell<Vec<Picked>> = const { RefCell::new(Vec::new()) };
    /// The editor's context, to wake it when something arrives.
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
    static UNSAVED: Cell<bool> = const { Cell::new(false) };
    /// Sessions whose tab is still open (they hold their lock); `None` until the browser said.
    static LIVE: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    /// Where the pointer is over the canvas while files are dragged (CSS pixels from its
    /// top-left corner), ADR 0059: egui is told only that something hovers.
    static DRAG_POS: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
    /// Dropped files read so far, by ticket: each file's name and bytes.
    static DROPPED: RefCell<Vec<(u64, ReadFiles)>> = const { RefCell::new(Vec::new()) };
    static TICKET: Cell<u64> = const { Cell::new(0) };
}

/// Starts the editor on the page's canvas.
pub fn start() {
    let mac = web_sys::window().and_then(|w| w.navigator().user_agent().ok()).is_some_and(|ua| ua.contains("Mac"));
    crate::platform::set_web_on_mac(mac);
    guard_leaving();
    wasm_bindgen_futures::spawn_local(async {
        match run().await {
            Ok(()) => {
                if let Some(el) = element(LOADING_ID) {
                    el.remove();
                }
            }
            Err(e) => {
                web_sys::console::error_1(&JsValue::from_str(&e));
                if let Some(el) = element(LOADING_ID) {
                    // i18n-exempt: the page's own loading text, before the interface (and its language) starts
                    el.set_text_content(Some(&format!("AniCraft could not start in this browser: {e}")));
                }
            }
        }
    });
}

async fn run() -> Result<(), String> {
    // i18n-exempt: for the console and the page's loading text, before the interface starts
    let canvas = element(CANVAS_ID).ok_or("the page has no canvas for AniCraft")?.dyn_into::<web_sys::HtmlCanvasElement>().map_err(|_| "the AniCraft element is not a canvas")?;
    // Text needs a face to lay out with, and a page can't see the computer's fonts.
    anicraft_engine::text::Fonts::register(fonts());
    let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
    track_drags(&canvas);
    eframe::WebRunner::new()
        .start(
            canvas,
            eframe::WebOptions::default(),
            Box::new(move |cc| {
                CTX.with(|c| *c.borrow_mut() = Some(cc.egui_ctx.clone()));
                let mut app = crate::app::App::new(cc, session, None);
                // The start screen, unless it was switched off (ADR 0059).
                if app.prefs.start_screen {
                    crate::start::open(&mut app);
                }
                Ok(Box::new(app))
            }),
        )
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
}

/// The embedded interface faces, or egui's own (in every build) when there are none.
fn fonts() -> Vec<Arc<[u8]>> {
    let mut faces = crate::theme::embedded_fonts();
    if faces.is_empty()
        && let Some(f) = egui::FontDefinitions::default().font_data.get("Ubuntu-Light") // i18n-exempt: a font key
    {
        faces.push(Arc::from(f.font.as_ref()));
    }
    faces
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn element(id: &str) -> Option<web_sys::Element> {
    document()?.get_element_by_id(id)
}

fn repaint() {
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            ctx.request_repaint();
        }
    });
}

// ----- files ------------------------------------------------------------------------------------

/// Opens the browser's file picker for `cmd`, showing files with these extensions. The file
/// arrives through [`take_picked`]; a cancelled picker delivers nothing.
pub fn pick_file(cmd: &'static str, exts: &[&str]) {
    let Some(doc) = document() else { return };
    let Some(body) = doc.body() else { return };
    // One picker at a time: a cancelled one is still on the page.
    if let Some(old) = doc.get_element_by_id(PICKER_ID) {
        old.remove();
    }
    let Some(input) = doc.create_element("input").ok().and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
    input.set_type("file");
    input.set_id(PICKER_ID);
    input.set_accept(&exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(","));
    let _ = input.style().set_property("display", "none");
    let picker = input.clone();
    let on_change = Closure::once_into_js(move || {
        let file = picker.files().and_then(|l| l.get(0));
        picker.remove();
        if let Some(file) = file {
            wasm_bindgen_futures::spawn_local(read_picked(cmd, file));
        }
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    // Some browsers only open pickers that are on the page.
    if body.append_child(&input).is_ok() {
        input.click();
    }
}

async fn read_picked(cmd: &'static str, file: web_sys::File) {
    let name = file.name();
    let data = if file.size() > MAX_PICK_BYTES {
        // Shown in the status bar: "Could not open {name}: {e}".
        Err(tr!("it is larger than {size} MB", size = MAX_PICK_BYTES as u64 >> 20))
    } else {
        match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
            Ok(buf) => Ok(js_sys::Uint8Array::new(&buf).to_vec()),
            Err(_) => Err(tr!("the browser could not read it").to_string()),
        }
    };
    PICKED.with(|p| p.borrow_mut().push(Picked { cmd, name, data }));
    repaint();
}

/// The next file the picker delivered.
pub fn take_picked() -> Option<Picked> {
    PICKED.with(|p| {
        let mut p = p.borrow_mut();
        (!p.is_empty()).then(|| p.remove(0))
    })
}

/// Keeps where the pointer is while files are dragged over the canvas (eframe's own listener
/// tells egui only that files hover, ADR 0059).
fn track_drags(canvas: &web_sys::HtmlCanvasElement) {
    let target = canvas.clone();
    let on_over = Closure::<dyn FnMut(web_sys::DragEvent)>::new(move |e: web_sys::DragEvent| {
        let r = target.get_bounding_client_rect();
        DRAG_POS.with(|p| p.set(Some((e.client_x() as f32 - r.left() as f32, e.client_y() as f32 - r.top() as f32))));
    });
    if canvas.add_event_listener_with_callback("dragover", on_over.as_ref().unchecked_ref()).is_ok() {
        on_over.forget();
    }
    let on_leave = Closure::<dyn FnMut(web_sys::DragEvent)>::new(|_e: web_sys::DragEvent| DRAG_POS.with(|p| p.set(None)));
    if canvas.add_event_listener_with_callback("dragleave", on_leave.as_ref().unchecked_ref()).is_ok() {
        on_leave.forget();
    }
}

/// Where the pointer was over the canvas during the drag, in CSS pixels.
pub fn drag_pos() -> Option<(f32, f32)> {
    DRAG_POS.with(Cell::get)
}

pub fn clear_drag_pos() {
    DRAG_POS.with(|p| p.set(None));
}

/// Reads dropped files' bytes; they arrive together through [`take_dropped`] with the ticket
/// returned here.
pub fn read_dropped(files: Vec<web_sys::File>) -> u64 {
    let ticket = TICKET.with(|t| {
        t.set(t.get().wrapping_add(1));
        t.get()
    });
    wasm_bindgen_futures::spawn_local(async move {
        let mut out = Vec::with_capacity(files.len());
        for file in files {
            let name = file.name();
            let data = if file.size() > MAX_PICK_BYTES {
                Err(tr!("it is larger than {size} MB", size = MAX_PICK_BYTES as u64 >> 20))
            } else {
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => Ok(js_sys::Uint8Array::new(&buf).to_vec()),
                    Err(_) => Err(tr!("the browser could not read it").to_string()),
                }
            };
            out.push((name, data));
        }
        DROPPED.with(|d| d.borrow_mut().push((ticket, out)));
        repaint();
    });
    ticket
}

/// The next dropped files the browser finished reading.
pub fn take_dropped() -> Option<(u64, ReadFiles)> {
    DROPPED.with(|d| {
        let mut d = d.borrow_mut();
        (!d.is_empty()).then(|| d.remove(0))
    })
}

/// Offers `data` as a download named after `path`'s file name.
pub fn download(path: &Path, data: &[u8]) {
    let name = path.file_name().map_or_else(|| "download".to_string(), |n| n.to_string_lossy().into_owned());
    if let Err(e) = try_download(&name, data) {
        web_sys::console::error_2(&JsValue::from_str(&format!("could not download {name}")), &e); // i18n-exempt: console
    }
}

/// Its errors go to the browser's console only.
fn try_download(name: &str, data: &[u8]) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?; // i18n-exempt: console text
    let doc = window.document().ok_or("no document")?; // i18n-exempt: console text
    let body = doc.body().ok_or("no body")?; // i18n-exempt: console text
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(data));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let a = doc.create_element("a")?.dyn_into::<web_sys::HtmlAnchorElement>()?;
    a.set_href(&url);
    a.set_download(name);
    let _ = a.style().set_property("display", "none");
    body.append_child(&a)?;
    a.click();
    a.remove();
    // The download has its own copy long before this.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 60_000)?;
    Ok(())
}

// ----- leaving the page -------------------------------------------------------------------------

pub fn set_unsaved(unsaved: bool) {
    UNSAVED.with(|u| u.set(unsaved));
}

/// The browser asks "Leave site?" while there are unsaved changes.
fn guard_leaving() {
    let Some(window) = web_sys::window() else { return };
    let on_leave = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(|e: web_sys::BeforeUnloadEvent| {
        if UNSAVED.with(Cell::get) {
            e.prevent_default();
            e.set_return_value(tr!("You have unsaved changes."));
        }
    });
    if window.add_event_listener_with_callback("beforeunload", on_leave.as_ref().unchecked_ref()).is_ok() {
        // Lives as long as the page.
        on_leave.forget();
    }
}

// ----- storage ----------------------------------------------------------------------------------

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn storage_get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

pub fn storage_set(key: &str, value: &str) -> Result<(), String> {
    let s = storage().ok_or(tr!("this browser keeps no storage for the site (private browsing?)"))?;
    s.set_item(key, value).map_err(|_| tr!("the browser's storage for this site is full (documents with large images or sounds may not fit)").to_string())
}

fn storage_remove(key: &str) {
    if let Some(s) = storage() {
        let _ = s.remove_item(key);
    }
}

fn storage_keys() -> Vec<String> {
    let Some(s) = storage() else { return Vec::new() };
    let n = s.length().unwrap_or(0).min(100_000);
    (0..n).filter_map(|i| s.key(i).ok().flatten()).collect()
}

// ----- crash recovery (ADR 0015 in a browser) ---------------------------------------------------

fn doc_key(id: &str) -> String {
    format!("{RECOVERY_PREFIX}{id}.doc")
}

fn meta_key(id: &str) -> String {
    format!("{RECOVERY_PREFIX}{id}.meta")
}

/// This tab's autosave: the same calls as the desktop's `Autosaver`, writing to browser storage.
/// Storage is synchronous, so a copy is written when it is asked for (serializing a typical
/// document takes milliseconds; the outcome comes back on the next poll, as on the desktop).
pub struct BrowserSaver {
    id: String,
    next_tag: u64,
    done: RefCell<Vec<Outcome>>,
}

impl BrowserSaver {
    pub fn start() -> Result<BrowserSaver, String> {
        storage().ok_or(tr!("this browser keeps no storage for the site (private browsing?)"))?;
        let millis = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let id = format!("web-{millis:x}-{:08x}", (js_sys::Math::random() * f64::from(u32::MAX)) as u32);
        hold_lock(&id);
        refresh_live();
        Ok(BrowserSaver { id, next_tag: 0, done: RefCell::new(Vec::new()) })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn write(&mut self, doc: Document, original: Option<&Path>) -> u64 {
        self.next_tag += 1;
        let tag = self.next_tag;
        let meta = RecoveryMeta::describe(&self.id, &doc, original);
        let t = web_time::Instant::now();
        let outcome = match anicraft_engine::format::to_json(&doc) {
            Ok(json) => {
                let serialize_ms = t.elapsed().as_secs_f64() * 1000.0;
                let t = web_time::Instant::now();
                // The description last: a description always has its whole document.
                let meta_json = serde_json::to_string(&meta).unwrap_or_default();
                match storage_set(&doc_key(&self.id), &json).and_then(|()| storage_set(&meta_key(&self.id), &meta_json)) {
                    Ok(()) => Outcome::Written { tag, stats: WriteStats { bytes: json.len() as u64, serialize_ms, write_ms: t.elapsed().as_secs_f64() * 1000.0 } },
                    Err(error) => Outcome::Failed { tag, error },
                }
            }
            Err(e) => Outcome::Failed { tag, error: e.to_string() },
        };
        self.done.borrow_mut().push(outcome);
        tag
    }

    pub fn clear(&mut self) -> u64 {
        self.next_tag += 1;
        discard(&self.id);
        self.done.borrow_mut().push(Outcome::Cleared { tag: self.next_tag });
        self.next_tag
    }

    pub fn poll(&self) -> Vec<Outcome> {
        std::mem::take(&mut *self.done.borrow_mut())
    }

    pub fn finish(mut self, last: Option<(Document, Option<PathBuf>)>) -> Vec<Outcome> {
        match last {
            Some((doc, original)) => self.write(doc, original.as_deref()),
            None => self.clear(),
        };
        self.poll()
    }
}

/// `navigator.locks`, where the browser has Web Locks (secure pages: https and localhost).
fn locks() -> Option<JsValue> {
    let nav = web_sys::window()?.navigator();
    js_sys::Reflect::get(&nav, &JsValue::from_str("locks")).ok().filter(|l| l.is_object())
}

fn method(obj: &JsValue, name: &str) -> Option<js_sys::Function> {
    js_sys::Reflect::get(obj, &JsValue::from_str(name)).ok()?.dyn_into::<js_sys::Function>().ok()
}

/// Holds this tab's lock until the tab goes (the callback's promise never settles).
fn hold_lock(id: &str) {
    let Some(locks) = locks() else { return };
    let Some(request) = method(&locks, "request") else { return };
    let hold = Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(|_| js_sys::Promise::new(&mut |_, _| {}));
    if request.call2(&locks, &JsValue::from_str(&format!("{LOCK_PREFIX}{id}")), hold.as_ref()).is_ok() {
        hold.forget();
    }
}

/// Asks the browser which sessions' tabs are open; the answer arrives asynchronously. Without
/// Web Locks every other session counts as closed (its copy is offered; restoring is a copy).
pub fn refresh_live() {
    LIVE.with(|l| *l.borrow_mut() = None);
    let query = locks().and_then(|locks| method(&locks, "query").and_then(|q| q.call0(&locks).ok())).and_then(|p| p.dyn_into::<js_sys::Promise>().ok());
    let Some(query) = query else {
        LIVE.with(|l| *l.borrow_mut() = Some(Vec::new()));
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let mut live = Vec::new();
        if let Ok(snapshot) = wasm_bindgen_futures::JsFuture::from(query).await
            && let Ok(held) = js_sys::Reflect::get(&snapshot, &JsValue::from_str("held"))
        {
            for info in js_sys::Array::from(&held).iter().take(10_000) {
                if let Some(name) = js_sys::Reflect::get(&info, &JsValue::from_str("name")).ok().and_then(|n| n.as_string())
                    && let Some(id) = name.strip_prefix(LOCK_PREFIX)
                {
                    live.push(id.to_string());
                }
            }
        }
        LIVE.with(|l| *l.borrow_mut() = Some(live));
        repaint();
    });
}

/// Copies left by tabs that closed or crashed, newest first, not `own`; `None` until the
/// browser has said which tabs are open.
pub fn scan(own: Option<&str>) -> Option<Vec<Entry>> {
    let live = LIVE.with(|l| l.borrow().clone())?;
    let mut found: Vec<Entry> = storage_keys()
        .iter()
        .filter_map(|k| k.strip_prefix(RECOVERY_PREFIX)?.strip_suffix(".meta"))
        .filter(|id| anicraft_engine::recovery::valid_id(id) && Some(*id) != own && !live.iter().any(|l| l == id))
        .filter_map(|id| {
            let meta = serde_json::from_str::<RecoveryMeta>(&storage_get(&meta_key(id))?).ok()?;
            let bytes = storage_get(&doc_key(id))?.len() as u64;
            Some(Entry { session: id.to_string(), document: PathBuf::new(), meta, bytes })
        })
        .collect();
    found.sort_by_key(|e| std::cmp::Reverse(e.meta.saved_at));
    Some(found)
}

/// The document and description of session `id`'s copy.
pub fn load(id: &str) -> Option<(String, RecoveryMeta)> {
    let meta = serde_json::from_str::<RecoveryMeta>(&storage_get(&meta_key(id))?).ok()?;
    Some((storage_get(&doc_key(id))?, meta))
}

/// Removes session `id`'s copy.
pub fn discard(id: &str) {
    storage_remove(&meta_key(id));
    storage_remove(&doc_key(id));
}

/// Opens a page served beside the editor (the user guide) in a new tab.
pub fn open_page(url: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| tr!("this page has no window").to_string())?;
    window.open_with_url_and_target(url, "_blank").map(|_| ()).map_err(|_| tr!("the browser didn't open {url}", url = url).to_string())
}
