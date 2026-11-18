//! Brücke zur Android-App (Kotlin, Zweig `android`): Die App zeigt diese Oberfläche in einer WebView und
//! stellt `window.NrzAndroid` bereit. Darüber
//!
//! - speichert die App Titel aufs Handy (`download`) – für die Offline-Bibliothek der App,
//!   die auch ohne Verbindung zum Server spielt,
//! - zeigt sie eine Medien-Benachrichtigung und hält die Wiedergabe im Hintergrund am Leben
//!   (`mediaState`); deren Knöpfe kommen über `window.nrzAndroidCommand("toggle" | "next" |
//!   "prev")` zurück,
//! - meldet sie gespeicherte Titel (`offlineKeys`) und Fortschritt
//!   (`window.nrzAndroidOffline(json)`),
//! - öffnet sie die Spotify-Anmeldung in einem eigenen Fenster (`spotifyLogin`) und gibt das
//!   Cookie `sp_dc` über `window.nrzAndroidSpotify(cookie | null)` zurück.
//!
//! Im Browser gibt es `NrzAndroid` nicht – dann tut hier alles nichts.

use crate::i18n::{tr, trf};
use crate::components::overlay::toast;
use crate::player::{self, Cmd, Ev};
use dioxus::prelude::*;
use js_sys::{Function, Reflect};
use nrzmusic_types::PlayableTrack;
use serde_json::json;
use std::collections::HashSet;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Auf dem Handy gespeicherte Titel (Schlüssel `local:<id>` / `yt:<videoId>`).
pub static OFFLINE: GlobalSignal<HashSet<String>> = Signal::global(HashSet::new);
/// Das Spotify-Anmeldefenster der App ist offen.
pub static SPOTIFY_LOGIN_OPEN: GlobalSignal<bool> = Signal::global(|| false);
/// Cookie `sp_dc` aus dem Anmeldefenster, noch nicht an den Server geschickt.
pub static SPOTIFY_COOKIE: GlobalSignal<Option<String>> = Signal::global(|| None);

fn bridge() -> Option<JsValue> {
    let w = web_sys::window()?;
    let b = Reflect::get(&w, &"NrzAndroid".into()).ok()?;
    (!b.is_undefined() && !b.is_null()).then_some(b)
}

/// Läuft die Oberfläche in der Android-App?
pub fn available() -> bool {
    bridge().is_some()
}

fn has(name: &str) -> bool {
    bridge().and_then(|b| Reflect::get(&b, &JsValue::from_str(name)).ok()).is_some_and(|f| f.is_function())
}

fn call(name: &str, args: &[JsValue]) -> Option<JsValue> {
    let b = bridge()?;
    let f = Reflect::get(&b, &JsValue::from_str(name)).ok()?.dyn_into::<Function>().ok()?;
    let arr = js_sys::Array::new();
    for a in args {
        arr.push(a);
    }
    Reflect::apply(&f, &b, &arr).ok()
}

fn origin() -> String {
    web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default()
}

fn absolute(url: &str) -> String {
    if url.starts_with('/') { format!("{}{url}", origin()) } else { url.to_string() }
}

/// Einmal beim Start: Rückruf für die Knöpfe der Benachrichtigung, gespeicherte Titel holen.
pub fn init() {
    if !available() {
        return;
    }
    let Some(w) = web_sys::window() else { return };
    let cmd = Closure::<dyn FnMut(JsValue)>::new(|v: JsValue| {
        match v.as_string().as_deref() {
            Some("toggle") => player::send(Ev::Cmd(Cmd::Toggle)),
            Some("play") => player::send(Ev::Cmd(Cmd::Play)),
            Some("pause") => player::send(Ev::Cmd(Cmd::Pause)),
            Some("next") => player::send(Ev::Cmd(Cmd::Next)),
            Some("prev") => player::send(Ev::Cmd(Cmd::Prev)),
            _ => {}
        }
    });
    let _ = Reflect::set(&w, &"nrzAndroidCommand".into(), cmd.as_ref());
    cmd.forget();
    let progress = Closure::<dyn FnMut(JsValue)>::new(|v: JsValue| {
        let Some(text) = v.as_string() else { return };
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        let key = ev["key"].as_str().unwrap_or_default().to_string();
        match ev["state"].as_str() {
            Some("done") => {
                OFFLINE.write().insert(key);
            }
            Some("removed") => {
                OFFLINE.write().remove(&key);
            }
            _ => {}
        }
    });
    let _ = Reflect::set(&w, &"nrzAndroidOffline".into(), progress.as_ref());
    progress.forget();
    let spotify = Closure::<dyn FnMut(JsValue)>::new(|v: JsValue| {
        *SPOTIFY_LOGIN_OPEN.write() = false;
        if let Some(cookie) = v.as_string().filter(|c| !c.is_empty()) {
            *SPOTIFY_COOKIE.write() = Some(cookie);
        }
    });
    let _ = Reflect::set(&w, &"nrzAndroidSpotify".into(), spotify.as_ref());
    spotify.forget();
    refresh();
}

/// Gespeicherte Titel neu von der App holen.
pub fn refresh() {
    if let Some(v) = call("offlineKeys", &[]).and_then(|v| v.as_string())
        && let Ok(keys) = serde_json::from_str::<Vec<String>>(&v)
    {
        *OFFLINE.write() = keys.into_iter().collect();
    }
}

pub fn is_offline(key: &str) -> bool {
    OFFLINE.read().contains(key)
}

/// Titel aufs Handy laden (die App lädt im Hintergrund, mit dem Cookie der Sitzung).
pub fn save(tracks: Vec<PlayableTrack>) {
    let items: Vec<_> = tracks
        .iter()
        .filter(|t| !is_offline(&t.key()))
        .map(|t| {
            json!({
                "key": t.key(),
                "url": absolute(&player::stream_url(t)),
                "title": t.title(),
                "artist": t.artist(),
                "album": t.album().unwrap_or_default(),
                "duration_ms": t.duration_ms().unwrap_or(0),
                "artwork": t.artwork_url(600).map(|u| absolute(&u)),
            })
        })
        .collect();
    if items.is_empty() {
        toast(tr("Schon auf dem Handy."));
        return;
    }
    let n = items.len();
    call("download", &[JsValue::from_str(&serde_json::Value::Array(items).to_string())]);
    toast(if n == 1 { tr("Wird aufs Handy geladen – danach unter „Offline“ in der App.") } else { trf("{0} Titel werden aufs Handy geladen.", &[&n]) });
}

pub fn remove(key: &str) {
    call("remove", &[JsValue::from_str(key)]);
}

/// Kann diese App Spotify selbst anmelden (ab 1.4)?
pub fn has_spotify_login() -> bool {
    has("spotifyLogin")
}

/// Spotify-Anmeldefenster der App öffnen; das Cookie kommt in [SPOTIFY_COOKIE] an.
pub fn spotify_login() {
    *SPOTIFY_LOGIN_OPEN.write() = true;
    call("spotifyLogin", &[]);
}

/// Offline-Bibliothek der App öffnen.
pub fn open_offline() {
    call("openOffline", &[]);
}

/// Wiedergabe an die App melden (Benachrichtigung, Hintergrund).
pub fn media_state(track: Option<&PlayableTrack>, playing: bool) {
    if !available() {
        return;
    }
    let state = match track {
        Some(t) => json!({
            "playing": playing,
            "title": t.title(),
            "artist": t.artist(),
            "artwork": t.artwork_url(300).map(|u| absolute(&u)),
        }),
        None => json!({ "playing": false }),
    };
    call("mediaState", &[JsValue::from_str(&state.to_string())]);
}
