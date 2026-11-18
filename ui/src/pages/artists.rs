//! Künstler als Raster und die Künstlerseite (beliebteste Titel, Alben).

use crate::api;
use crate::components::cards::*;
use crate::components::common::*;
use crate::components::tracks::{TrackList, pick_playlist};
use crate::i18n::{tr, trf};
use crate::pages::{LibraryTabs, LoadError};
use crate::state::LIBRARY_VERSION;
use crate::util;
use dioxus::prelude::*;
use nrzmusic_types::{Artist, ArtistDetail, PlayableTrack, cover_url};

#[component]
pub fn Artists() -> Element {
    let mut filter = use_signal(String::new);
    let list = use_resource(move || async move {
        let _ = LIBRARY_VERSION();
        let q = filter();
        if !q.is_empty() {
            gloo_timers::future::TimeoutFuture::new(200).await;
        }
        api::get::<Vec<Artist>>(&format!("/artists?q={}", api::enc(&q))).await
    });
    rsx! {
        LibraryTabs {}
        PageHeader { title: tr("Künstler"),
            description: match &*list.read() { Some(Ok(l)) if filter().is_empty() => trf("{0} Künstler", &[&l.len()]), _ => String::new() },
            actions: rsx! {
                input { class: tw_merge::tw_merge!(INPUT, "w-40 sm:w-44"), r#type: "search", placeholder: tr("Filtern …"), value: "{filter}", oninput: move |e| filter.set(e.value()) }
            }
        }
        match &*list.read() {
            None => rsx! { Loading {} },
            Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
            Some(Ok(l)) if l.is_empty() => rsx! {
                EmptyState { title: if filter().is_empty() { tr("Noch keine Künstler") } else { tr("Nichts gefunden") }, icon: rsx! { icons::MicVocal {} } }
            },
            Some(Ok(l)) => rsx! {
                div { class: GRID,
                    for a in l.iter().cloned() {
                        ArtistCard { key: "{a.id}", artist: a }
                    }
                }
            },
        }
    }
}

#[component]
pub fn ArtistPage(id: String) -> Element {
    let mut all = use_signal(|| false);
    let detail = use_resource(use_reactive!(|id| async move {
        let _ = LIBRARY_VERSION();
        api::get::<ArtistDetail>(&format!("/artists/{}", api::enc(&id))).await
    }));
    match &*detail.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
        Some(Ok(d)) => {
            let a = d.artist.clone();
            let tracks: Vec<PlayableTrack> = d.tracks.iter().cloned().map(PlayableTrack::Local).collect();
            let shown: Vec<PlayableTrack> = if all() { tracks.clone() } else { tracks.iter().take(10).cloned().collect() };
            let cover = a.cover.as_ref().map(|c| cover_url(c, 600));
            let meta = format!(
                "{} · {} · {}",
                if a.album_count == 1 { tr("1 Album") } else { trf("{0} Alben", &[&a.album_count]) },
                util::tracks_count(a.track_count as u64),
                util::long_duration(d.tracks.iter().map(|t| t.duration_ms).sum())
            );
            let refs: Vec<_> = tracks.iter().map(|t| t.to_ref()).collect();
            let more = tracks.len() > 10;
            rsx! {
                DetailHeader {
                    image: rsx! { Cover { src: cover.clone(), round: true, class: "aspect-square w-full", eager: true, icon: rsx! { icons::MicVocal {} } } },
                    backdrop: cover.clone(),
                    kind: tr("Künstler"),
                    title: a.name.clone(),
                    meta,
                    PlayButtons { tracks: tracks.clone() }
                    button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Playlist hinzufügen …"), onclick: move |_| pick_playlist(refs.clone()),
                        icons::ListPlus { class: "size-5" }
                    }
                }
                section { class: "mb-10",
                    h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Beliebteste Titel")} }
                    TrackList { tracks: shown }
                    if more {
                        button { class: "{btn(BTN_GHOST)} {BTN_SM} mt-2", onclick: move |_| all.toggle(),
                            if all() { {tr("Weniger anzeigen")} } else { {trf("Alle {0} Titel anzeigen", &[&tracks.len()])} }
                        }
                    }
                }
                if !d.albums.is_empty() {
                    section {
                        h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Alben")} }
                        div { class: GRID,
                            for al in d.albums.iter().cloned() {
                                AlbumCard { key: "{al.id}", album: al }
                            }
                        }
                    }
                }
            }
        }
    }
}
