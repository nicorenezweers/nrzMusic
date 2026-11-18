//! Teilen: Link für einen Titel, die eigenen Likes oder eine Playlist anlegen
//! (`POST /api/v1/shares`) und in einem Fenster zeigen – kopieren, über das Teilen-Menü des
//! Geräts schicken oder öffnen. Der Link funktioniert ohne Konto (Seite `/s/<token>`).

use crate::api;
use crate::components::common::*;
use crate::components::overlay::{Modal, toast_error};
use crate::i18n::tr;
use crate::pages::spotify::CopyField;
use crate::util;
use dioxus::prelude::*;
use nrzmusic_types::{PlayableTrack, ShareInfo, ShareKind, ShareRequest};

struct Pending {
    label: String,
    link: Option<String>,
}

static DIALOG: GlobalSignal<Option<Pending>> = Signal::global(|| None);

/// Vollständige Adresse eines Links.
pub fn full_url(path: &str) -> String {
    let origin = web_sys::window().and_then(|w| w.location().origin().ok()).unwrap_or_default();
    format!("{origin}{path}")
}

fn open(req: ShareRequest, label: String) {
    *DIALOG.write() = Some(Pending { label, link: None });
    spawn(async move {
        match api::post::<ShareInfo, _>("/shares", &req).await {
            Ok(info) => {
                if let Some(p) = DIALOG.write().as_mut() {
                    p.link = Some(full_url(&info.path));
                }
            }
            Err(e) => {
                *DIALOG.write() = None;
                toast_error(e);
            }
        }
    });
}

pub fn share_track(t: &PlayableTrack) {
    open(ShareRequest { kind: ShareKind::Track, track: Some(t.to_ref()), playlist_id: None }, format!("{} – {}", t.title(), t.artist()));
}

pub fn share_favorites() {
    open(ShareRequest { kind: ShareKind::Favorites, track: None, playlist_id: None }, tr("Meine Likes"));
}

pub fn share_playlist(id: i64, name: &str) {
    open(ShareRequest { kind: ShareKind::Playlist, track: None, playlist_id: Some(id) }, name.to_string());
}

/// Teilen-Menü des Geräts (iPhone, Android-Browser, Safari/Chrome auf dem Desktop).
fn native_share_available() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w.navigator(), &"share".into()).ok())
        .is_some_and(|f| f.is_function())
}

fn native_share(title: &str, url: &str) {
    use wasm_bindgen::JsCast;
    let Some(w) = web_sys::window() else { return };
    let nav = w.navigator();
    let Some(f) = js_sys::Reflect::get(&nav, &"share".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) else { return };
    let data = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&data, &"title".into(), &title.into());
    let _ = js_sys::Reflect::set(&data, &"url".into(), &url.into());
    if let Ok(p) = f.call1(&nav, &data)
        && let Ok(p) = p.dyn_into::<js_sys::Promise>()
    {
        // Abbrechen im Menü ist kein Fehler
        let ignore = wasm_bindgen::closure::Closure::once(|_: wasm_bindgen::JsValue| {});
        let _ = p.catch(&ignore);
        ignore.forget();
    }
}

/// Das Fenster (einmal im Rahmen der App).
#[component]
pub fn ShareDialogHost() -> Element {
    let mut open_sig = use_signal(|| false);
    let state = DIALOG.read();
    let is_open = state.is_some();
    use_effect(move || {
        let o = DIALOG.read().is_some();
        if *open_sig.peek() != o {
            open_sig.set(o);
        }
    });
    // Mit Esc/Klick daneben geschlossen
    use_effect(move || {
        if !open_sig() && DIALOG.peek().is_some() {
            *DIALOG.write() = None;
        }
    });
    let Some(p) = state.as_ref().filter(|_| is_open) else { return rsx! {} };
    let label = p.label.clone();
    let link = p.link.clone();
    rsx! {
        Modal { open: open_sig, title: tr("Teilen"), description: label.clone(),
            div { class: "mt-4 grid gap-4",
                match link.clone() {
                    None => rsx! { Loading { text: tr("Link wird erstellt …") } },
                    Some(url) => {
                        let (u1, u2, l1) = (url.clone(), url.clone(), label.clone());
                        rsx! {
                            CopyField { value: url.clone() }
                            p { class: "text-xs text-muted-foreground", {tr("Wer den Link hat, kann die Titel ansehen und anhören – auch ohne Konto. Likes und Playlists zeigen immer den aktuellen Stand. Unter Einstellungen → Geteilte Links lässt er sich wieder löschen.")} }
                            div { class: "flex flex-wrap justify-end gap-2",
                                button { class: "{btn(BTN_OUTLINE)} rounded-full", onclick: move |_| util::open_external(&u1),
                                    icons::ExternalLink { class: "size-4" }
                                    {tr("Öffnen")}
                                }
                                if native_share_available() {
                                    button { class: "{btn(BTN_PRIMARY)} rounded-full", onclick: move |_| native_share(&l1, &u2),
                                        icons::Share2 { class: "size-4" }
                                        {tr("Teilen …")}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
