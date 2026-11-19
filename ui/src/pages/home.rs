//! Startseite: Begrüssung nach Tageszeit, oben „Dein Mix“ (Titel aus „Für dich“ abwechselnd
//! mit deinen Favoriten – Abspielen läuft endlos weiter), dann „Zuletzt hinzugefügt“,
//! „Meistgespielt“, „Entdecken“. Was zuletzt lief, steht im Verlauf, nicht hier.
//! Leere Bibliothek → grosse Ablagefläche mit Erklärung.

use crate::api;
use crate::components::cards::*;
use crate::components::common::*;
use crate::components::tracks::TrackList;
use crate::i18n::{tr, trf};
use crate::pages::LoadError;
use crate::pages::library::UploadButtons;
use crate::state::{self, HISTORY_VERSION, LIBRARY_VERSION, STATUS};
use crate::{Route, util};
use dioxus::prelude::*;
use nrzmusic_types::{FeedPage, FeedRequest, HomeData, PlayableTrack};

#[component]
pub fn Home() -> Element {
    let data = use_resource(move || async move {
        let _ = (LIBRARY_VERSION(), HISTORY_VERSION());
        api::get::<HomeData>("/home").await
    });
    rsx! {
        div { class: "flex flex-col gap-9",
            h1 { class: "text-2xl font-bold tracking-tight md:text-3xl", {util::greeting()} }
            MixShelf {}
            match &*data.read() {
                None => rsx! { Loading {} },
                Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
                Some(Ok(d)) => rsx! { HomeContent { data: d.clone() } },
            }
        }
    }
}

#[component]
fn HomeContent(data: HomeData) -> Element {
    let empty = data.recently_added.is_empty() && data.most_played.is_empty() && data.random_albums.is_empty();
    let most: Vec<PlayableTrack> = data.most_played.iter().cloned().map(PlayableTrack::Local).collect();
    let counts: Vec<String> = data.most_played.iter().map(|t| trf("{0}× gespielt", &[&t.play_count])).collect();
    rsx! {
        if empty {
            EmptyLibrary {}
        }
        if !data.recently_added.is_empty() {
            Shelf { title: tr("Zuletzt hinzugefügt"),
                action: rsx! { Link { class: "text-sm text-muted-foreground hover:text-foreground", to: Route::Albums {}, {tr("Alle Alben")} } },
                for a in data.recently_added.iter().cloned() {
                    AlbumCard { key: "{a.id}", album: a, class: SHELF_ITEM }
                }
            }
        }
        if !most.is_empty() {
            section {
                div { class: "mb-3 flex items-end justify-between gap-3",
                    h2 { class: "text-lg font-semibold tracking-tight", {tr("Meistgespielt")} }
                    Link { class: "text-sm text-muted-foreground hover:text-foreground", to: Route::Songs {}, {tr("Alle Songs")} }
                }
                TrackList { tracks: most, extra: counts }
            }
        }
        if !data.random_albums.is_empty() {
            Shelf { title: tr("Entdecken"),
                for a in data.random_albums.iter().cloned() {
                    AlbumCard { key: "{a.id}", album: a, class: SHELF_ITEM }
                }
            }
        }
        if state::youtube_enabled() {
            Link { class: "group flex items-center gap-4 rounded-xl border bg-card p-4 transition-colors hover:bg-accent/40", to: Route::YouTube { q: String::new(), tab: String::new() },
                div { class: "grid size-12 shrink-0 place-items-center rounded-xl bg-muted", icons::Youtube { class: "size-6" } }
                div { class: "min-w-0 flex-1",
                    p { class: "font-medium", {tr("YouTube Music entdecken")} }
                    p { class: "text-sm text-muted-foreground", {tr("Suchen, abspielen, in Playlists mischen – ohne Konto.")} }
                }
                icons::ChevronRight { class: "size-5 text-muted-foreground transition-transform group-hover:translate-x-0.5" }
            }
        }
    }
}

/// Leere Bibliothek: erklärt den Musikordner und nimmt Dateien an.
#[component]
pub fn EmptyLibrary() -> Element {
    let dir = STATUS.read().as_ref().map(|s| s.music_dir.clone()).unwrap_or_else(|| "/music".into());
    rsx! {
        div { class: "flex flex-col items-center gap-5 rounded-3xl border-2 border-dashed px-6 py-14 text-center md:py-20",
            div { class: "grid size-16 place-items-center rounded-2xl bg-muted", icons::FolderUp { class: "size-8" } }
            div { class: "max-w-lg",
                h2 { class: "text-xl font-semibold tracking-tight md:text-2xl", {tr("Deine Bibliothek ist noch leer")} }
                p { class: "mt-2 text-sm text-muted-foreground",
                    {tr("Lege Musikdateien (MP3, FLAC, M4A, OGG, Opus, WAV …) in den Musikordner – nrzMusic findet sie von selbst. Oder zieh Dateien und ganze Ordner hierher.")}
                }
                p { class: "mt-3",
                    code { class: "rounded-md bg-muted px-2 py-1 font-mono text-sm", "{dir}" }
                }
            }
            div { class: "flex flex-wrap justify-center gap-2", UploadButtons {} }
        }
    }
}

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let _ = segments;
    rsx! {
        EmptyState { title: tr("Seite nicht gefunden"), description: tr("Diese Adresse gibt es nicht (mehr)."), icon: rsx! { icons::Compass {} },
            Link { class: btn(BTN_OUTLINE), to: Route::Home {}, {tr("Zur Startseite")} }
        }
    }
}

/// „Dein Mix“: „Für dich“ und Favoriten abwechselnd, die Favoriten zufällig ausgewählt.
async fn load_mix() -> Vec<PlayableTrack> {
    let (feed, favs) = futures_util::future::join(
        api::post::<FeedPage, _>("/feed", &FeedRequest { exclude: Vec::new(), count: Some(16) }),
        api::get::<Vec<PlayableTrack>>("/favorites"),
    )
    .await;
    let feed: Vec<PlayableTrack> = feed.map(|p| p.items.into_iter().map(|i| i.track).collect()).unwrap_or_default();
    let mut favs = favs.unwrap_or_default();
    // Zufällig mischen (Fisher-Yates)
    for i in (1..favs.len()).rev() {
        let j = (js_sys::Math::random() * (i + 1) as f64) as usize;
        favs.swap(i, j.min(i));
    }
    mix(feed, favs, 24)
}

/// Abwechselnd aus beiden Listen, ohne doppelte Titel, höchstens `max`.
fn mix(a: Vec<PlayableTrack>, b: Vec<PlayableTrack>, max: usize) -> Vec<PlayableTrack> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let (mut a, mut b) = (a.into_iter(), b.into_iter());
    loop {
        let (x, y) = (a.next(), b.next());
        if x.is_none() && y.is_none() {
            break;
        }
        for t in [x, y].into_iter().flatten() {
            if out.len() < max && seen.insert(t.key()) {
                out.push(t);
            }
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

#[component]
fn MixShelf() -> Element {
    let mut mix = use_resource(move || async move {
        let _ = crate::state::FAVORITES_VERSION();
        load_mix().await
    });
    let list = mix.read().clone().unwrap_or_default();
    if mix.read().is_some() && list.is_empty() {
        return rsx! {};
    }
    let play = list.clone();
    rsx! {
        section { class: "min-w-0",
            div { class: "mb-3 flex flex-wrap items-end justify-between gap-3",
                div {
                    h2 { class: "text-lg font-semibold tracking-tight", {tr("Dein Mix")} }
                    p { class: "text-sm text-muted-foreground", {tr("Für dich und deine Favoriten – läuft endlos weiter.")} }
                }
                div { class: "flex items-center gap-2",
                    button { class: "{btn(BTN_PRIMARY)} rounded-full", disabled: list.is_empty(), onclick: move |_| crate::player::play_feed(play.clone(), 0),
                        icons::Play { class: "size-4 fill-current" } {tr("Abspielen")}
                    }
                    button { class: "{btn(BTN_GHOST)} rounded-full", title: tr("Neu mischen"), onclick: move |_| mix.restart(),
                        icons::Shuffle { class: "size-4" }
                    }
                }
            }
            if list.is_empty() {
                Loading {}
            } else {
                div { class: "shelf -mx-4 flex gap-3 overflow-x-auto px-4 pb-2 md:-mx-6 md:gap-4 md:px-6",
                    for (i , t) in list.iter().enumerate() {
                        TrackCard { key: "{t.key()}", list: list.clone(), index: i, class: SHELF_ITEM }
                    }
                }
            }
        }
    }
}
