//! Startseite: Begrüssung nach Tageszeit, Reihen "Zuletzt hinzugefügt", "Zuletzt gehört",
//! "Meistgespielt", "Entdecken". Leere Bibliothek → grosse Ablagefläche mit Erklärung.

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
use nrzmusic_types::{HomeData, PlayableTrack};

#[component]
pub fn Home() -> Element {
    let data = use_resource(move || async move {
        let _ = (LIBRARY_VERSION(), HISTORY_VERSION());
        api::get::<HomeData>("/home").await
    });
    rsx! {
        div { class: "flex flex-col gap-9",
            h1 { class: "text-2xl font-bold tracking-tight md:text-3xl", {util::greeting()} }
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
    let played = data.recently_played.clone();
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
        if !played.is_empty() {
            Shelf { title: tr("Zuletzt gehört"),
                action: rsx! { Link { class: "text-sm text-muted-foreground hover:text-foreground", to: Route::History {}, {tr("Verlauf")} } },
                for (i , t) in played.iter().enumerate() {
                    TrackCard { key: "{t.key()}", list: played.clone(), index: i, class: SHELF_ITEM }
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
