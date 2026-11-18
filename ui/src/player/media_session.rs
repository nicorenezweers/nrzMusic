//! Media Session API (Sperrbildschirm, Medientasten, Kopfhörer): Metadaten mit Cover und
//! die Aktionen play/pause/previoustrack/nexttrack/seekto – über `js_sys::Reflect`, weil
//! `navigator.mediaSession` in web-sys nur instabil verfügbar ist.

use super::{Cmd, Ev, send};
use js_sys::{Array, Function, Object, Reflect};
use nrzmusic_types::PlayableTrack;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

type Handler = Closure<dyn FnMut(JsValue)>;
type Action = (&'static str, Box<dyn FnMut(JsValue)>);

thread_local! {
    static HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
    /// Zuletzt gemeldeter Titel (für die Benachrichtigung der Android-App)
    static LAST: RefCell<Option<PlayableTrack>> = const { RefCell::new(None) };
}

fn session() -> Option<JsValue> {
    let nav = web_sys::window()?.navigator();
    let s = Reflect::get(&nav, &"mediaSession".into()).ok()?;
    if s.is_undefined() || s.is_null() { None } else { Some(s) }
}

fn set(target: &JsValue, key: &str, value: &JsValue) {
    let _ = Reflect::set(target, &JsValue::from_str(key), value);
}

pub fn init() {
    let Some(s) = session() else { return };
    let Some(set_handler) = Reflect::get(&s, &"setActionHandler".into()).ok().and_then(|f| f.dyn_into::<Function>().ok()) else {
        return;
    };
    let actions: Vec<Action> = vec![
        ("play", Box::new(|_| send(Ev::Cmd(Cmd::Play)))),
        ("pause", Box::new(|_| send(Ev::Cmd(Cmd::Pause)))),
        ("previoustrack", Box::new(|_| send(Ev::Cmd(Cmd::Prev)))),
        ("nexttrack", Box::new(|_| send(Ev::Cmd(Cmd::Next)))),
        (
            "seekto",
            Box::new(|d: JsValue| {
                if let Some(t) = Reflect::get(&d, &"seekTime".into()).ok().and_then(|v| v.as_f64()) {
                    send(Ev::Cmd(Cmd::SeekTo(t)));
                }
            }),
        ),
        (
            "seekbackward",
            Box::new(|d: JsValue| {
                let off = Reflect::get(&d, &"seekOffset".into()).ok().and_then(|v| v.as_f64()).unwrap_or(10.0);
                send(Ev::Cmd(Cmd::SeekBy(-off)));
            }),
        ),
        (
            "seekforward",
            Box::new(|d: JsValue| {
                let off = Reflect::get(&d, &"seekOffset".into()).ok().and_then(|v| v.as_f64()).unwrap_or(10.0);
                send(Ev::Cmd(Cmd::SeekBy(off)));
            }),
        ),
    ];
    HANDLERS.with(|h| {
        let mut h = h.borrow_mut();
        for (name, f) in actions {
            let c = Closure::<dyn FnMut(JsValue)>::new(f);
            // Nicht jeder Browser kennt jede Aktion – Fehler dann ignorieren
            let _ = set_handler.call2(&s, &JsValue::from_str(name), c.as_ref());
            h.push(c);
        }
    });
}

/// Titel, Künstler, Album und Cover an das Betriebssystem melden.
/// Titel des Browser-Tabs: "Titel – nrzMusic" bzw. nur "nrzMusic".
fn set_document_title(track: Option<&PlayableTrack>) {
    let Some(doc) = crate::util::document() else { return };
    let title = match track {
        Some(t) if !t.title().trim().is_empty() => format!("{} – nrzMusic", t.title().trim()),
        _ => "nrzMusic".to_string(),
    };
    if doc.title() != title {
        doc.set_title(&title);
    }
}

pub fn set_metadata(track: Option<&PlayableTrack>) {
    set_document_title(track);
    LAST.with(|l| *l.borrow_mut() = track.cloned());
    let Some(s) = session() else { return };
    let Some(track) = track else {
        set(&s, "metadata", &JsValue::NULL);
        return;
    };
    let Some(ctor) = web_sys::window()
        .and_then(|w| Reflect::get(&w, &"MediaMetadata".into()).ok())
        .and_then(|f| f.dyn_into::<Function>().ok())
    else {
        return;
    };
    let init = Object::new();
    set(&init, "title", &track.title().into());
    set(&init, "artist", &track.artist().into());
    set(&init, "album", &track.album().unwrap_or_default().into());
    let artwork = Array::new();
    let origin = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
    let sizes: &[u32] = match track {
        PlayableTrack::Local(_) => &[96, 300, 600],
        PlayableTrack::Youtube(_) => &[0],
    };
    for size in sizes {
        if let Some(url) = track.artwork_url(*size) {
            let src = if url.starts_with('/') { format!("{origin}{url}") } else { url };
            let a = Object::new();
            set(&a, "src", &src.into());
            if *size > 0 {
                set(&a, "sizes", &format!("{size}x{size}").into());
                set(&a, "type", &"image/jpeg".into());
            }
            artwork.push(&a);
        }
    }
    set(&init, "artwork", &artwork);
    if let Ok(meta) = Reflect::construct(&ctor, &Array::of1(&init)) {
        set(&s, "metadata", &meta);
    }
}

pub fn set_playing(playing: bool) {
    LAST.with(|l| crate::android::media_state(l.borrow().as_ref(), playing));
    if let Some(s) = session() {
        set(&s, "playbackState", &(if playing { "playing" } else { "paused" }).into());
    }
}

/// Position für die Fortschrittsanzeige des Systems.
pub fn set_position(position: f64, duration: f64) {
    let Some(s) = session() else { return };
    if !(duration.is_finite() && duration > 0.0) {
        return;
    }
    let Some(f) = Reflect::get(&s, &"setPositionState".into()).ok().and_then(|f| f.dyn_into::<Function>().ok()) else { return };
    let state = Object::new();
    set(&state, "duration", &duration.into());
    set(&state, "playbackRate", &1.0.into());
    set(&state, "position", &position.clamp(0.0, duration).into());
    let _ = f.call1(&s, &state);
}
