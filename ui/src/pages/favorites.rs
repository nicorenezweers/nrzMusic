//! Favoriten und Verlauf (lokal und YouTube gemischt).

use crate::api;
use crate::components::common::*;
use crate::components::download::DownloadYoutubeItem;
use crate::components::overlay::{Align, Dropdown};
use crate::components::tracks::{Lead, TrackList, pick_playlist};
use crate::i18n::tr;
use crate::pages::{LibraryTabs, LoadError};
use crate::state::{FAVORITES_VERSION, HISTORY_VERSION, LIBRARY_VERSION};
use crate::util;
use dioxus::prelude::*;
use nrzmusic_types::PlayableTrack;

#[component]
pub fn Favorites() -> Element {
    let list = use_resource(move || async move {
        let _ = (FAVORITES_VERSION(), LIBRARY_VERSION());
        api::get::<Vec<PlayableTrack>>("/favorites").await
    });
    rsx! {
        LibraryTabs {}
        match &*list.read() {
            None => rsx! { PageHeader { title: tr("Favoriten") } Loading {} },
            Some(Err(e)) => rsx! { PageHeader { title: tr("Favoriten") } LoadError { message: e.to_string() } },
            Some(Ok(l)) => {
                let refs: Vec<_> = l.iter().map(|t| t.to_ref()).collect();
                rsx! {
                    PageHeader { title: tr("Favoriten"),
                        description: util::tracks_count(l.len() as u64),
                        actions: rsx! {
                            PlayButtons { tracks: l.clone() }
                            button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Alle zu einer Playlist hinzufügen …"), disabled: l.is_empty(), onclick: move |_| pick_playlist(refs.clone()),
                                icons::ListPlus { class: "size-5" }
                            }
                            button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Meine Likes teilen …"), disabled: l.is_empty(), onclick: move |_| crate::share::share_favorites(),
                                icons::Share2 { class: "size-5" }
                            }
                            if l.iter().any(|t| matches!(t, PlayableTrack::Youtube(_))) && crate::downloads::available() {
                                Dropdown { title: tr("Mehr"), align: Align::End,
                                    trigger_class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full",
                                    trigger: rsx! { icons::Ellipsis { class: "size-5" } },
                                    DownloadYoutubeItem { tracks: l.clone() }
                                }
                            }
                        }
                    }
                    if l.is_empty() {
                        EmptyState { title: tr("Noch keine Favoriten"), description: tr("Tippe bei einem Titel auf das Herz – lokal oder von YouTube Music."), icon: rsx! { icons::Heart {} } }
                    } else {
                        TrackList { tracks: l.clone() }
                    }
                }
            }
        }
    }
}

#[component]
pub fn History() -> Element {
    let list = use_resource(move || async move {
        let _ = (HISTORY_VERSION(), LIBRARY_VERSION());
        api::get::<Vec<PlayableTrack>>("/history?limit=300").await
    });
    rsx! {
        LibraryTabs {}
        match &*list.read() {
            None => rsx! { PageHeader { title: tr("Verlauf") } Loading {} },
            Some(Err(e)) => rsx! { PageHeader { title: tr("Verlauf") } LoadError { message: e.to_string() } },
            Some(Ok(l)) => {
                let extra: Vec<String> = l.iter().map(|t| match t {
                    PlayableTrack::Local(x) => x.last_played.map(util::datetime).unwrap_or_default(),
                    PlayableTrack::Youtube(_) => String::new(),
                }).collect();
                rsx! {
                    PageHeader { title: tr("Verlauf"),
                        description: tr("Zuletzt gehört, neueste zuerst."),
                        actions: rsx! { PlayButtons { tracks: l.clone() } }
                    }
                    if l.is_empty() {
                        EmptyState { title: tr("Noch nichts gehört"), description: tr("Ein Titel landet hier nach 30 Sekunden (oder der halben Länge)."), icon: rsx! { icons::History {} } }
                    } else {
                        TrackList { tracks: l.clone(), lead: Lead::Cover, extra }
                    }
                }
            }
        }
    }
}
