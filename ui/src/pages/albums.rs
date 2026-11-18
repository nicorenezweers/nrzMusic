//! Alben als Raster (sortierbar, filterbar) und die Albumseite.

use crate::api;
use crate::components::cards::*;
use crate::components::common::*;
use crate::components::tracks::{Lead, TrackList, pick_playlist};
use crate::i18n::tr;
use crate::pages::{LibraryTabs, LoadError};
use crate::state::LIBRARY_VERSION;
use crate::{Route, player, util};
use dioxus::prelude::*;
use nrzmusic_types::{Album, AlbumDetail, PlayableTrack, cover_url};

#[component]
pub fn Albums() -> Element {
    let mut sort = use_signal(|| util::storage_get("nrzmusic-albums-sort").unwrap_or_else(|| "title".into()));
    let mut filter = use_signal(String::new);
    let list = use_resource(move || async move {
        let _ = LIBRARY_VERSION();
        let q = filter();
        if !q.is_empty() {
            gloo_timers::future::TimeoutFuture::new(200).await;
        }
        api::get::<Vec<Album>>(&format!("/albums?q={}&sort={}", api::enc(&q), sort())).await
    });
    rsx! {
        LibraryTabs {}
        PageHeader { title: tr("Alben"),
            description: match &*list.read() { Some(Ok(l)) if filter().is_empty() => crate::i18n::trf("{0} Alben", &[&l.len()]), _ => String::new() },
            actions: rsx! {
                input { class: tw_merge::tw_merge!(INPUT, "w-40 sm:w-44"), r#type: "search", placeholder: tr("Filtern …"), value: "{filter}", oninput: move |e| filter.set(e.value()) }
                select { class: tw_merge::tw_merge!(SELECT, "w-auto"), value: "{sort}",
                    onchange: move |e| {
                        let v = e.value();
                        util::storage_set("nrzmusic-albums-sort", &v);
                        sort.set(v);
                    },
                    for (v , l) in [("title", tr("Titel")), ("artist", tr("Künstler")), ("year", tr("Jahr")), ("added", tr("Hinzugefügt"))] {
                        option { value: v, selected: sort() == v, "{l}" }
                    }
                }
            }
        }
        match &*list.read() {
            None => rsx! { Loading {} },
            Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
            Some(Ok(l)) if l.is_empty() => rsx! {
                EmptyState { title: if filter().is_empty() { tr("Noch keine Alben") } else { tr("Nichts gefunden") }, icon: rsx! { icons::Disc3 {} } }
            },
            Some(Ok(l)) => rsx! {
                div { class: GRID,
                    for a in l.iter().cloned() {
                        AlbumCard { key: "{a.id}", album: a }
                    }
                }
            },
        }
    }
}

#[component]
pub fn AlbumPage(id: String) -> Element {
    let detail = use_resource(use_reactive!(|id| async move {
        let _ = LIBRARY_VERSION();
        api::get::<AlbumDetail>(&format!("/albums/{}", api::enc(&id))).await
    }));
    match &*detail.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! { LoadError { message: e.to_string() } },
        Some(Ok(d)) => {
            let a = d.album.clone();
            let tracks: Vec<PlayableTrack> = d.tracks.iter().cloned().map(PlayableTrack::Local).collect();
            let cover = a.cover.as_ref().map(|c| cover_url(c, 600));
            let mut meta = Vec::new();
            if let Some(y) = a.year {
                meta.push(y.to_string());
            }
            meta.push(util::tracks_count(d.tracks.len() as u64));
            meta.push(util::long_duration(d.tracks.iter().map(|t| t.duration_ms).sum()));
            let t_queue = tracks.clone();
            let refs: Vec<_> = tracks.iter().map(|t| t.to_ref()).collect();
            rsx! {
                DetailHeader {
                    image: rsx! { Cover { src: cover.clone(), class: "aspect-square w-full rounded-xl", eager: true, icon: rsx! { icons::Disc3 {} } } },
                    backdrop: cover.clone(),
                    kind: tr("Album"),
                    title: a.title.clone(),
                    subtitle: rsx! {
                        if !a.artist_id.is_empty() {
                            Link { class: "font-medium hover:underline", to: Route::ArtistPage { id: a.artist_id.clone() }, "{a.artist}" }
                        } else {
                            span { class: "font-medium", "{a.artist}" }
                        }
                    },
                    meta: meta.join(" · "),
                    PlayButtons { tracks: tracks.clone() }
                    button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Warteschlange"), onclick: move |_| player::add_to_queue(t_queue.clone()),
                        icons::ListEnd { class: "size-5" }
                    }
                    button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Playlist hinzufügen …"), onclick: move |_| pick_playlist(refs.clone()),
                        icons::ListPlus { class: "size-5" }
                    }
                }
                TrackList { tracks, lead: Lead::Number, show_album: false }
            }
        }
    }
}
