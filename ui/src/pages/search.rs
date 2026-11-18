//! Suche in der eigenen Bibliothek (Songs, Alben, Künstler) – mit Sprung zur selben Suche
//! auf YouTube Music.

use crate::api;
use crate::components::cards::*;
use crate::components::common::*;
use crate::components::tracks::TrackList;
use crate::i18n::{tr, trf};
use crate::pages::LoadError;
use crate::state::{self, LIBRARY_VERSION};
use crate::{Route, util};
use dioxus::prelude::*;
use nrzmusic_types::{LibrarySearch, PlayableTrack};

#[component]
pub fn Search(q: String) -> Element {
    let mut all_tracks = use_signal(|| false);
    let query = q.clone();
    let results = use_resource(use_reactive!(|query| async move {
        let _ = LIBRARY_VERSION();
        if query.trim().is_empty() {
            return Ok(LibrarySearch::default());
        }
        // Beim Tippen nicht jede Taste einzeln suchen
        gloo_timers::future::TimeoutFuture::new(180).await;
        api::get::<LibrarySearch>(&format!("/search?q={}&limit=50", api::enc(query.trim()))).await
    }));
    let yt = state::youtube_enabled();
    let q_yt = q.clone();
    rsx! {
        div { class: "mb-6 flex flex-wrap items-center gap-3",
            h1 { class: "text-2xl font-semibold tracking-tight md:text-3xl",
                if q.trim().is_empty() { {tr("Suche")} } else { {trf("„{0}“", &[&q.trim()])} }
            }
            if yt && !q.trim().is_empty() {
                Link { class: "{btn(BTN_OUTLINE)} {BTN_SM} ml-auto rounded-full", to: Route::YouTube { q: q_yt.trim().to_string(), tab: String::new() },
                    icons::Youtube { class: "size-4" }
                    {tr("Auf YouTube Music suchen")}
                }
            }
        }
        if q.trim().is_empty() {
            EmptyState { title: tr("Wonach suchst du?"), description: tr("Titel, Alben und Künstler in deiner Bibliothek – Tippen genügt. Taste / springt ins Suchfeld."), icon: rsx! { icons::Search {} },
                button { class: btn(BTN_OUTLINE), onclick: move |_| util::focus("global-search"), {tr("Zum Suchfeld")} }
            }
        } else {
            match &*results.read() {
                None => rsx! { Loading {} },
                Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
                Some(Ok(r)) if r.tracks.is_empty() && r.albums.is_empty() && r.artists.is_empty() => rsx! {
                    EmptyState { title: tr("In der Bibliothek nichts gefunden"), icon: rsx! { icons::Search {} },
                        if yt {
                            Link { class: btn(BTN_PRIMARY), to: Route::YouTube { q: q.trim().to_string(), tab: String::new() },
                                icons::Youtube { class: "size-4" } {tr("Auf YouTube Music suchen")}
                            }
                        }
                    }
                },
                Some(Ok(r)) => {
                    let tracks: Vec<PlayableTrack> = r.tracks.iter().cloned().map(PlayableTrack::Local).collect();
                    let shown: Vec<PlayableTrack> = if all_tracks() { tracks.clone() } else { tracks.iter().take(8).cloned().collect() };
                    let more = tracks.len() > 8;
                    rsx! {
                        div { class: "flex flex-col gap-10",
                            if !tracks.is_empty() {
                                section {
                                    div { class: "mb-3 flex items-center justify-between gap-3",
                                        h2 { class: "text-lg font-semibold tracking-tight", {tr("Songs")} }
                                        div { class: "flex gap-2", PlayButtons { tracks: tracks.clone() } }
                                    }
                                    TrackList { tracks: shown }
                                    if more {
                                        button { class: "{btn(BTN_GHOST)} {BTN_SM} mt-2", onclick: move |_| all_tracks.toggle(),
                                            if all_tracks() { {tr("Weniger anzeigen")} } else { {trf("Alle {0} Songs anzeigen", &[&tracks.len()])} }
                                        }
                                    }
                                }
                            }
                            if !r.albums.is_empty() {
                                section {
                                    h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Alben")} }
                                    div { class: GRID,
                                        for a in r.albums.iter().cloned() { AlbumCard { key: "{a.id}", album: a } }
                                    }
                                }
                            }
                            if !r.artists.is_empty() {
                                section {
                                    h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Künstler")} }
                                    div { class: GRID,
                                        for a in r.artists.iter().cloned() { ArtistCard { key: "{a.id}", artist: a } }
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
