//! Hochladen in den Musikordner: Dateien und ganze Ordner (Drag & Drop mit
//! `webkitGetAsEntry`, "Ordner wählen" mit `webkitdirectory`). Jede Datei geht einzeln per
//! XmlHttpRequest an `POST /api/v1/upload` (Feld `files`; der Dateiname trägt den Pfad im
//! Ordner, z. B. `Album/01 Song.mp3`) – so gibt es Fortschritt pro Datei.

use crate::components::overlay::{toast, toast_error};
use crate::i18n::{tr, trf};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use futures_channel::oneshot;
use futures_util::future::{Either, select};
use nrzmusic_types::UploadResult;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Dateiendungen, die überhaupt hochgeladen werden (der Server prüft noch einmal).
const AUDIO: &[&str] = &["mp3", "flac", "m4a", "m4b", "mp4", "aac", "alac", "ogg", "oga", "opus", "wav", "wave", "aif", "aiff", "aifc", "ape", "wv", "mpc", "wma", "dsf"];

#[derive(Clone, Debug, PartialEq)]
pub enum UpState {
    Waiting,
    Sending,
    Imported(String),
    Skipped(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Upload {
    pub id: u64,
    pub name: String,
    pub size: f64,
    pub loaded: f64,
    pub state: UpState,
}

impl Upload {
    pub fn finished(&self) -> bool {
        !matches!(self.state, UpState::Waiting | UpState::Sending)
    }
}

/// Alle Uploads dieser Sitzung (für "Ordner & Upload" und die Fortschrittsanzeige).
pub static UPLOADS: GlobalSignal<Vec<Upload>> = Signal::global(Vec::new);
/// Dateien werden über das Fenster gezogen.
pub static DRAGGING: GlobalSignal<bool> = Signal::global(|| false);

thread_local! {
    static FILES: RefCell<HashMap<u64, web_sys::File>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
    static RUNNING: Cell<bool> = const { Cell::new(false) };
    /// Zeitpunkt (ms) des letzten fertigen Durchgangs
    static FINISHED_AT: Cell<f64> = const { Cell::new(0.0) };
}

/// Läuft gerade ein Upload oder ist einer eben fertig geworden? (Dann meldet der Upload
/// selbst, was neu ist – keine zweite Meldung aus den Live-Ereignissen.)
pub fn busy_recently() -> bool {
    RUNNING.with(|r| r.get()) || js_sys::Date::now() - FINISHED_AT.with(|f| f.get()) < 15_000.0
}

fn is_audio(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| AUDIO.contains(&ext.to_ascii_lowercase().as_str()))
}

fn hidden(name: &str) -> bool {
    name.split('/').any(|part| part.starts_with('.'))
}

/// Dateien (mit Pfad im Ordner) einreihen und hochladen.
pub fn enqueue(files: Vec<(web_sys::File, String)>) {
    let files: Vec<_> = files.into_iter().filter(|(_, n)| !hidden(n)).collect();
    if files.is_empty() {
        return;
    }
    let mut ignored = 0;
    {
        let mut ups = UPLOADS.write();
        // Erledigte aus früheren Durchgängen wegräumen, wenn nichts mehr läuft
        if ups.iter().all(Upload::finished) {
            ups.clear();
        }
        for (file, name) in files {
            let id = NEXT.with(|n| {
                let v = n.get();
                n.set(v + 1);
                v
            });
            let size = file.size();
            if is_audio(&name) {
                FILES.with(|f| f.borrow_mut().insert(id, file));
                ups.push(Upload { id, name, size, loaded: 0.0, state: UpState::Waiting });
            } else {
                ignored += 1;
                ups.push(Upload { id, name, size, loaded: 0.0, state: UpState::Skipped(tr("kein Audioformat")) });
            }
        }
    }
    if ignored > 0 && UPLOADS.peek().iter().all(Upload::finished) {
        toast_error(tr("Keine Musikdateien gefunden."));
        return;
    }
    if !RUNNING.with(|r| r.replace(true)) {
        spawn_forever(worker());
    }
}

fn set_state(id: u64, f: impl FnOnce(&mut Upload)) {
    if let Some(u) = UPLOADS.write().iter_mut().find(|u| u.id == id) {
        f(u);
    }
}

async fn worker() {
    loop {
        let next = UPLOADS.peek().iter().find(|u| u.state == UpState::Waiting).map(|u| (u.id, u.name.clone()));
        let Some((id, name)) = next else { break };
        let Some(file) = FILES.with(|f| f.borrow_mut().remove(&id)) else {
            set_state(id, |u| u.state = UpState::Failed(tr("Datei nicht mehr verfügbar")));
            continue;
        };
        set_state(id, |u| u.state = UpState::Sending);
        let progress = Rc::new(Cell::new(0.0));
        let result = match start(&file, &name, progress.clone()) {
            Ok((rx, _keep)) => {
                let mut rx = rx;
                loop {
                    let tick = gloo_timers::future::TimeoutFuture::new(150);
                    match select(rx, tick).await {
                        Either::Left((res, _)) => break res.unwrap_or_else(|_| Err(tr("abgebrochen"))),
                        Either::Right((_, pending)) => {
                            rx = pending;
                            let loaded = progress.get();
                            set_state(id, |u| u.loaded = loaded);
                        }
                    }
                }
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(res) => {
                let state = if let Some(t) = res.imported.first() {
                    UpState::Imported(t.title.clone())
                } else if let Some(s) = res.skipped.first() {
                    UpState::Skipped(crate::i18n::tr(&s.reason))
                } else {
                    UpState::Skipped(tr("nichts übernommen"))
                };
                set_state(id, |u| {
                    u.loaded = u.size;
                    u.state = state;
                });
            }
            Err(e) => set_state(id, |u| u.state = UpState::Failed(e)),
        }
    }
    RUNNING.with(|r| r.set(false));
    FINISHED_AT.with(|f| f.set(js_sys::Date::now()));
    // Zusammenfassung
    let ups = UPLOADS.peek().clone();
    let imported = ups.iter().filter(|u| matches!(u.state, UpState::Imported(_))).count();
    let skipped = ups.iter().filter(|u| matches!(u.state, UpState::Skipped(_))).count();
    let failed = ups.iter().filter(|u| matches!(u.state, UpState::Failed(_))).count();
    if failed > 0 {
        toast_error(trf("{0} importiert, {1} übersprungen, {2} fehlgeschlagen.", &[&imported, &skipped, &failed]));
    } else if skipped > 0 {
        toast(trf("{0} importiert, {1} übersprungen.", &[&imported, &skipped]));
    } else {
        toast(if imported == 1 { tr("1 Titel importiert.") } else { trf("{0} Titel importiert.", &[&imported]) });
    }
    crate::state::bump_library();
}

type Done = Result<UploadResult, String>;
type Keep = (web_sys::XmlHttpRequest, Vec<Closure<dyn FnMut(web_sys::ProgressEvent)>>);

/// Eine Datei per XmlHttpRequest schicken; Fortschritt landet in `progress`.
fn start(file: &web_sys::File, name: &str, progress: Rc<Cell<f64>>) -> Result<(oneshot::Receiver<Done>, Keep), String> {
    let err = |_| tr("Upload nicht möglich");
    let xhr = web_sys::XmlHttpRequest::new().map_err(err)?;
    xhr.open_with_async("POST", "/api/v1/upload", true).map_err(err)?;
    xhr.set_request_header("Accept", "application/json").map_err(err)?;
    let form = web_sys::FormData::new().map_err(err)?;
    form.append_with_blob_and_filename("files", file, name).map_err(err)?;

    let (tx, rx) = oneshot::channel::<Done>();
    let tx = Rc::new(RefCell::new(Some(tx)));
    let finish = {
        let tx = tx.clone();
        move |r: Done| {
            if let Some(s) = tx.borrow_mut().take() {
                let _ = s.send(r);
            }
        }
    };

    let on_progress = Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |e: web_sys::ProgressEvent| progress.set(e.loaded()));
    let x = xhr.clone();
    let f = finish.clone();
    let on_load = Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |_| {
        let status = x.status().unwrap_or(0);
        let text = x.response_text().ok().flatten().unwrap_or_default();
        if (200..300).contains(&status) {
            f(serde_json::from_str::<UploadResult>(&text).map_err(|e| e.to_string()));
        } else {
            let msg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["error"].as_str().map(crate::i18n::tr))
                .unwrap_or_else(|| trf("Fehler {0}", &[&status]));
            f(Err(msg));
        }
    });
    let f = finish;
    let on_error = Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |_| f(Err(tr("Netzwerkfehler"))));
    if let Ok(up) = xhr.upload() {
        up.set_onprogress(Some(on_progress.as_ref().unchecked_ref()));
    }
    xhr.set_onload(Some(on_load.as_ref().unchecked_ref()));
    xhr.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    xhr.send_with_opt_form_data(Some(&form)).map_err(err)?;
    Ok((rx, (xhr, vec![on_progress, on_load, on_error])))
}

/// Alle erledigten Einträge entfernen.
pub fn clear_finished() {
    UPLOADS.write().retain(|u| !u.finished());
}

// ------------------------------------------------------------------ Drag & Drop

/// Aus einem Drop-Ereignis (synchron, solange es läuft) die Einträge holen – Ordner über
/// `webkitGetAsEntry`, sonst die Dateien selbst.
pub fn take_drop(e: &DragEvent) -> (Vec<web_sys::FileSystemEntry>, Vec<web_sys::File>) {
    let Some(dt) = e.data().downcast::<web_sys::DragEvent>().and_then(|d| d.data_transfer()) else {
        return (Vec::new(), Vec::new());
    };
    let mut entries = Vec::new();
    let items = dt.items();
    for i in 0..items.length() {
        if let Some(item) = items.get(i) {
            if item.kind() != "file" {
                continue;
            }
            if let Ok(Some(entry)) = item.webkit_get_as_entry() {
                entries.push(entry);
            }
        }
    }
    if !entries.is_empty() {
        return (entries, Vec::new());
    }
    let files = dt.files().map(|l| (0..l.length()).filter_map(|i| l.get(i)).collect()).unwrap_or_default();
    (Vec::new(), files)
}

/// Werden gerade Dateien gezogen (nicht Text oder Links)?
pub fn has_files(e: &DragEvent) -> bool {
    // Hochladen dürfen nur Admins (bzw. alle ohne Konten)
    if crate::account::enabled() && !crate::account::is_admin() {
        return false;
    }
    e.data()
        .downcast::<web_sys::DragEvent>()
        .and_then(|d| d.data_transfer())
        .is_some_and(|dt| dt.types().iter().any(|t| t.as_string().as_deref() == Some("Files")))
}

async fn promise(p: js_sys::Promise) -> Result<JsValue, JsValue> {
    wasm_bindgen_futures::JsFuture::from(p).await
}

async fn entry_file(entry: &web_sys::FileSystemFileEntry) -> Option<web_sys::File> {
    let e = entry.clone();
    let p = js_sys::Promise::new(&mut |resolve, reject| e.file_with_callback_and_callback(&resolve, &reject));
    promise(p).await.ok()?.dyn_into().ok()
}

async fn read_dir(reader: &web_sys::FileSystemDirectoryReader) -> Vec<web_sys::FileSystemEntry> {
    let r = reader.clone();
    let p = js_sys::Promise::new(&mut |resolve, reject| {
        if r.read_entries_with_callback_and_callback(&resolve, &reject).is_err() {
            let _ = reject.call0(&JsValue::NULL);
        }
    });
    match promise(p).await {
        Ok(v) => js_sys::Array::from(&v).iter().filter_map(|e| e.dyn_into().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Ordner rekursiv auflösen; Pfade relativ zum gezogenen Element (`Album/01 Song.mp3`).
pub async fn expand(entries: Vec<web_sys::FileSystemEntry>) -> Vec<(web_sys::File, String)> {
    let mut out = Vec::new();
    let mut stack = entries;
    while let Some(entry) = stack.pop() {
        if entry.is_directory() {
            let Ok(dir) = entry.dyn_into::<web_sys::FileSystemDirectoryEntry>() else { continue };
            let reader = dir.create_reader();
            // readEntries liefert in Portionen, bis eine leere kommt
            loop {
                let batch = read_dir(&reader).await;
                if batch.is_empty() {
                    break;
                }
                stack.extend(batch);
            }
        } else if entry.is_file() {
            let path = entry.full_path().trim_start_matches('/').to_string();
            let Ok(fe) = entry.dyn_into::<web_sys::FileSystemFileEntry>() else { continue };
            if let Some(f) = entry_file(&fe).await {
                let name = if path.is_empty() { f.name() } else { path };
                out.push((f, name));
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Ganzer Ablauf nach einem Drop: Ordner auflösen, dann hochladen.
pub fn handle_drop(e: &DragEvent) {
    let (entries, files) = take_drop(e);
    if entries.is_empty() {
        enqueue(files.into_iter().map(|f| {
            let n = f.name();
            (f, n)
        }).collect());
        return;
    }
    spawn_forever(async move {
        let files = expand(entries).await;
        if files.is_empty() {
            toast_error(tr("Keine Dateien gefunden."));
        }
        enqueue(files);
    });
}

/// Fortschritt als Karte bei den Meldungen, solange hochgeladen wird.
#[component]
pub fn UploadToast() -> Element {
    let ups = UPLOADS.read();
    let active: Vec<&Upload> = ups.iter().filter(|u| !u.finished()).collect();
    if active.is_empty() {
        return rsx! {};
    }
    let total = ups.iter().filter(|u| !matches!(&u.state, UpState::Skipped(r) if r == &tr("kein Audioformat"))).count();
    let done = total - active.len();
    let size: f64 = ups.iter().map(|u| u.size).sum::<f64>().max(1.0);
    let loaded: f64 = ups.iter().map(|u| if u.finished() { u.size } else { u.loaded }).sum();
    let pct = (loaded / size * 100.0).clamp(0.0, 100.0);
    let current = active.iter().find(|u| u.state == UpState::Sending).map(|u| u.name.clone()).unwrap_or_default();
    rsx! {
        div { class: "pointer-events-auto rounded-xl border bg-popover px-4 py-3 text-sm text-popover-foreground shadow-lg animate-toast-in",
            div { class: "flex items-center gap-3",
                icons::Upload { class: "size-4 shrink-0 text-muted-foreground" }
                span { class: "flex-1 font-medium", {trf("Hochladen: {0} von {1}", &[&(done + 1).min(total), &total])} }
                span { class: "tabular-nums text-xs text-muted-foreground", "{pct:.0} %" }
            }
            div { class: "mt-2 h-1.5 overflow-hidden rounded-full bg-muted",
                div { class: "h-full rounded-full bg-primary transition-[width]", style: "width: {pct}%" }
            }
            if !current.is_empty() {
                p { class: "mt-1.5 truncate text-xs text-muted-foreground", "{current}" }
            }
        }
    }
}

/// Bildschirmfüllende Fläche beim Ziehen von Dateien über das Fenster.
#[component]
pub fn DropOverlay() -> Element {
    if !DRAGGING() {
        return rsx! {};
    }
    rsx! {
        div {
            class: "fixed inset-0 z-[72] flex items-center justify-center bg-background/80 p-6 backdrop-blur-sm animate-in",
            ondragover: move |e| e.prevent_default(),
            ondragleave: move |_| *DRAGGING.write() = false,
            ondrop: move |e| {
                e.prevent_default();
                *DRAGGING.write() = false;
                handle_drop(&e);
            },
            div { class: "pointer-events-none flex w-full max-w-xl flex-col items-center gap-4 rounded-3xl border-2 border-dashed border-foreground/30 px-8 py-16 text-center",
                div { class: "grid size-16 place-items-center rounded-2xl bg-muted",
                    icons::Music { class: "size-8" }
                }
                p { class: "text-2xl font-semibold tracking-tight", {tr("Musik hier ablegen")} }
                p { class: "max-w-sm text-sm text-muted-foreground",
                    {tr("Dateien und ganze Ordner landen im Musikordner und erscheinen gleich in der Bibliothek.")}
                }
            }
        }
    }
}
