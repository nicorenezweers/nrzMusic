//! Live-Ereignisse des Servers (`EventSource` auf `/api/v1/events`). Die Rückrufe des
//! Browsers laufen ausserhalb von Dioxus; sie schicken die Ereignisse über einen Kanal an
//! eine Aufgabe in der App, die dann die Signale ändert.

use crate::components::overlay::toast;
use crate::i18n::{tr, trf};
use crate::state::{self, FAVORITES_VERSION, SCAN};
use dioxus::prelude::ReadableExt;
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use nrzmusic_types::{ScanProgress, ServerEvent};
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

enum Msg {
    Open,
    Event(ServerEvent),
    /// Verbindung endgültig zu – nach einer Pause neu verbinden
    Closed,
}

struct Conn {
    source: web_sys::EventSource,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

thread_local! {
    static CONN: RefCell<Option<Conn>> = const { RefCell::new(None) };
}

fn connect(tx: UnboundedSender<Msg>) {
    let Ok(source) = web_sys::EventSource::new("/api/v1/events") else {
        let _ = tx.unbounded_send(Msg::Closed);
        return;
    };
    let t = tx.clone();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let _ = t.unbounded_send(Msg::Open);
    });
    let t = tx.clone();
    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
        if let Some(text) = e.data().as_string()
            && let Ok(ev) = serde_json::from_str::<ServerEvent>(&text) {
                let _ = t.unbounded_send(Msg::Event(ev));
            }
    });
    let t = tx;
    let src = source.clone();
    let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        // CONNECTING: der Browser versucht es selbst wieder; CLOSED: selbst neu verbinden
        if src.ready_state() == web_sys::EventSource::CLOSED {
            let _ = t.unbounded_send(Msg::Closed);
        }
    });
    source.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    source.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    source.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    CONN.with(|c| {
        if let Some(old) = c.borrow_mut().replace(Conn { source, _on_open: on_open, _on_message: on_message, _on_error: on_error }) {
            old.source.close();
        }
    });
}

/// Einmal beim Start (in der App) aufrufen; läuft für immer.
pub async fn run() {
    let (tx, mut rx) = unbounded::<Msg>();
    connect(tx.clone());
    let mut first = true;
    while let Some(msg) = rx.next().await {
        match msg {
            Msg::Open => {
                *state::LIVE.write() = true;
                // Nach einer Unterbrechung könnte etwas verpasst worden sein
                if !first {
                    state::bump_library();
                    state::bump_playlists();
                    state::load_favorites().await;
                    crate::downloads::load().await;
                    crate::spotify::load_status().await;
                }
                first = false;
                state::load_status().await;
            }
            Msg::Closed => {
                *state::LIVE.write() = false;
                CONN.with(|c| {
                    if let Some(old) = c.borrow_mut().take() {
                        old.source.close();
                    }
                });
                gloo_timers::future::TimeoutFuture::new(5000).await;
                connect(tx.clone());
            }
            Msg::Event(ev) => handle(ev).await,
        }
    }
}

async fn handle(ev: ServerEvent) {
    match ev {
        ServerEvent::ScanStarted => {
            *SCAN.write() = Some(ScanProgress::default());
        }
        ServerEvent::ScanProgress(p) => {
            *SCAN.write() = Some(p);
        }
        ServerEvent::ScanFinished { added, .. } => {
            *SCAN.write() = None;
            if added > 0 {
                toast(if added == 1 { tr("1 neuer Titel") } else { trf("{0} neue Titel", &[&added]) });
            }
            state::bump_library();
            state::load_status().await;
        }
        ServerEvent::LibraryChanged { tracks } => {
            // Neue Dateien im Ordner (nicht während eines Scans oder eigenen Uploads): kurz melden
            let before = state::STATUS.peek().as_ref().map(|s| s.tracks);
            if let Some(b) = before.filter(|b| tracks > *b && SCAN.peek().is_none() && !crate::upload::busy_recently()) {
                let n = tracks - b;
                toast(if n == 1 { tr("1 neuer Titel") } else { trf("{0} neue Titel", &[&n]) });
            }
            state::bump_library();
            state::load_status().await;
        }
        ServerEvent::PlaylistsChanged => state::bump_playlists(),
        ServerEvent::FavoritesChanged => {
            state::load_favorites().await;
            *FAVORITES_VERSION.write() += 1;
        }
        ServerEvent::DownloadUpdate(job) => crate::downloads::on_update(*job),
        ServerEvent::SpotifyChanged => crate::spotify::load_status().await,
        ServerEvent::SpotifyProgress { task, done, total } => crate::spotify::on_progress(task, done, total),
    }
}
