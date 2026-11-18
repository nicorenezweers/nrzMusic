//! Kacheln für Raster und Reihen: Alben, Künstler, Playlists, einzelne Titel – lokal und
//! von YouTube Music.

use crate::api;
use crate::components::common::*;
use crate::components::download::DownloadBadge;
use crate::components::overlay::toast_error;
use crate::components::tracks::{ArtistLinks, client_xy, open_menu};
use crate::i18n::tr;
use crate::player;
use crate::util;
use crate::Route;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::*;

/// Raster für Kacheln.
pub const GRID: &str = "grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 2xl:grid-cols-7";
/// Breite einer Kachel in einer waagrechten Reihe.
pub const SHELF_ITEM: &str = "w-36 shrink-0 sm:w-44";

/// Runder Play-Knopf, der beim Darüberfahren über einem Cover erscheint.
#[component]
fn HoverPlay(onclick: EventHandler<MouseEvent>) -> Element {
    rsx! {
        button {
            class: "absolute right-2 bottom-2 grid size-10 translate-y-1 place-items-center rounded-full bg-primary text-primary-foreground opacity-0 shadow-lg transition-all group-hover:translate-y-0 group-hover:opacity-100 focus-visible:opacity-100 hover:scale-105 max-md:hidden",
            title: tr("Abspielen"),
            onclick: move |e: MouseEvent| {
                e.prevent_default();
                e.stop_propagation();
                onclick.call(e);
            },
            icons::Play { class: "size-4 fill-current pointer-events-none" }
        }
    }
}

#[component]
fn CardText(#[props(into)] title: String, children: Element) -> Element {
    rsx! {
        div { class: "mt-2 min-w-0",
            p { class: "truncate text-sm font-medium", title: "{title}", "{title}" }
            p { class: "truncate text-xs text-muted-foreground", {children} }
        }
    }
}

/// Lokales Album.
#[component]
pub fn AlbumCard(album: Album, #[props(into, default)] class: String) -> Element {
    let id = album.id.clone();
    let sub = match album.year {
        Some(y) if !album.artist.is_empty() => format!("{y} · {}", album.artist),
        Some(y) => y.to_string(),
        None => album.artist.clone(),
    };
    rsx! {
        Link { class: "group block min-w-0 {class}", to: Route::AlbumPage { id: album.id.clone() },
            div { class: "relative",
                Cover { src: album.cover.as_ref().map(|c| cover_url(c, 300)), class: "aspect-square w-full rounded-lg", icon: rsx! { icons::Disc3 {} } }
                HoverPlay {
                    onclick: move |_| {
                        let id = id.clone();
                        spawn_forever(async move {
                            match api::get::<AlbumDetail>(&format!("/albums/{}", api::enc(&id))).await {
                                Ok(d) => player::play_now(d.tracks.into_iter().map(PlayableTrack::Local).collect(), 0),
                                Err(e) => toast_error(e),
                            }
                        });
                    }
                }
            }
            CardText { title: album.title.clone(), "{sub}" }
        }
    }
}

/// Lokaler Künstler.
#[component]
pub fn ArtistCard(artist: Artist, #[props(into, default)] class: String) -> Element {
    rsx! {
        Link { class: "group block min-w-0 text-center {class}", to: Route::ArtistPage { id: artist.id.clone() },
            Cover { src: artist.cover.as_ref().map(|c| cover_url(c, 300)), round: true, class: "aspect-square w-full transition-transform group-hover:scale-[1.02]", icon: rsx! { icons::MicVocal {} } }
            CardText { title: artist.name.clone(),
                {format!("{} · {}", if artist.album_count == 1 { tr("1 Album") } else { crate::i18n::trf("{0} Alben", &[&artist.album_count]) }, util::tracks_count(artist.track_count as u64))}
            }
        }
    }
}

/// Playlist mit Collage.
#[component]
pub fn PlaylistCard(playlist: PlaylistSummary, #[props(into, default)] class: String) -> Element {
    let id = playlist.id;
    rsx! {
        Link { class: "group block min-w-0 {class}", to: Route::PlaylistPage { id },
            div { class: "relative",
                Collage { artwork: playlist.artwork.clone(), class: "aspect-square w-full rounded-lg" }
                if playlist.item_count > 0 {
                    HoverPlay {
                        onclick: move |_| {
                            spawn_forever(async move {
                                match api::get::<Playlist>(&format!("/playlists/{id}")).await {
                                    Ok(p) => player::play_now(p.items, 0),
                                    Err(e) => toast_error(e),
                                }
                            });
                        }
                    }
                }
            }
            CardText { title: playlist.name.clone(),
                if playlist.youtube_playlist_id.is_some() { "YouTube · " }
                {util::tracks_count(playlist.item_count as u64)}
            }
        }
    }
}

/// Einzelner Titel als Kachel (Reihen auf Start- und YouTube-Seite). Klick spielt `list` ab `index`.
#[component]
pub fn TrackCard(list: Vec<PlayableTrack>, index: usize, #[props(into, default)] class: String) -> Element {
    let Some(t) = list.get(index).cloned() else { return rsx! {} };
    let current = player::current_key().as_deref() == Some(t.key().as_str());
    let playing = current && player::PLAYING();
    let t_menu = t.clone();
    rsx! {
        div { class: "group min-w-0 {class}",
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let (x, y) = client_xy(&e);
                open_menu(x, y, t_menu.clone(), Vec::new());
            },
            button { class: "relative block w-full cursor-pointer text-left",
                title: "{t.title()}",
                onclick: move |_| if current { player::toggle() } else { player::play_now(list.clone(), index) },
                Cover { src: t.artwork_url(300), class: "aspect-square w-full rounded-lg" }
                div { class: if current { "absolute inset-0 grid place-items-center rounded-lg bg-black/40" } else { "absolute inset-0 grid place-items-center rounded-lg bg-black/40 opacity-0 transition-opacity group-hover:opacity-100" },
                    div { class: "grid size-11 place-items-center rounded-full bg-primary text-primary-foreground shadow-lg",
                        if playing { icons::Pause { class: "size-4 fill-current" } } else { icons::Play { class: "size-4 fill-current" } }
                    }
                }
            }
            div { class: "mt-2 min-w-0",
                p { class: "truncate text-sm font-medium", "{t.title()}" }
                div { class: "flex min-w-0 items-center gap-1 text-xs text-muted-foreground",
                    if let PlayableTrack::Youtube(y) = &t {
                        DownloadBadge { video_id: y.video_id.clone() }
                    }
                    ArtistLinks { track: t.clone() }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ YouTube Music

fn yt_artists(a: &[YtArtistRef]) -> String {
    let names: Vec<&str> = a.iter().map(|x| x.name.as_str()).collect();
    names.join(", ")
}

#[component]
pub fn YtAlbumCard(album: YtAlbumSummary, #[props(into, default)] class: String) -> Element {
    let sub = [Some(album.kind.clone()).filter(|k| !k.is_empty()), album.year.clone(), Some(yt_artists(&album.artists)).filter(|a| !a.is_empty())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let id = album.browse_id.clone();
    rsx! {
        Link { class: "group block min-w-0 {class}", to: Route::YtAlbumPage { id: album.browse_id.clone() },
            div { class: "relative",
                Cover { src: album.thumbnail.clone(), class: "aspect-square w-full rounded-lg", icon: rsx! { icons::Disc3 {} } }
                HoverPlay {
                    onclick: move |_| {
                        let id = id.clone();
                        spawn_forever(async move {
                            match api::get::<YtAlbum>(&format!("/yt/album/{}", api::enc(&id))).await {
                                Ok(a) => player::play_now(a.tracks.into_iter().map(PlayableTrack::Youtube).collect(), 0),
                                Err(e) => toast_error(e),
                            }
                        });
                    }
                }
            }
            CardText { title: album.title.clone(), "{sub}" }
        }
    }
}

#[component]
pub fn YtArtistCard(artist: YtArtistSummary, #[props(into, default)] class: String) -> Element {
    rsx! {
        Link { class: "group block min-w-0 text-center {class}", to: Route::YtArtistPage { id: artist.browse_id.clone() },
            Cover { src: artist.thumbnail.clone(), round: true, class: "aspect-square w-full transition-transform group-hover:scale-[1.02]", icon: rsx! { icons::MicVocal {} } }
            CardText { title: artist.name.clone(), {artist.subscribers.clone().unwrap_or_else(|| tr("Künstler"))} }
        }
    }
}

#[component]
pub fn YtPlaylistCard(playlist: YtPlaylistSummary, #[props(into, default)] class: String) -> Element {
    let sub = [playlist.author.clone(), playlist.track_count.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let id = playlist.playlist_id.clone();
    rsx! {
        Link { class: "group block min-w-0 {class}", to: Route::YtPlaylistPage { id: playlist.playlist_id.clone() },
            div { class: "relative",
                Cover { src: playlist.thumbnail.clone(), class: "aspect-square w-full rounded-lg", icon: rsx! { icons::ListMusic {} } }
                HoverPlay {
                    onclick: move |_| {
                        let id = id.clone();
                        spawn_forever(async move {
                            match api::get::<YtPlaylist>(&format!("/yt/playlist/{}", api::enc(&id))).await {
                                Ok(p) => player::play_now(p.tracks.into_iter().map(PlayableTrack::Youtube).collect(), 0),
                                Err(e) => toast_error(e),
                            }
                        });
                    }
                }
            }
            CardText { title: playlist.title.clone(), "{sub}" }
        }
    }
}

/// Eintrag eines YouTube-Regals; Titel im selben Regal bilden zusammen die Warteschlange.
#[component]
pub fn YtItemCard(item: YtItem, shelf_tracks: Vec<PlayableTrack>, #[props(into, default)] class: String) -> Element {
    match item {
        YtItem::Track(t) => {
            let key = format!("yt:{}", t.video_id);
            let index = shelf_tracks.iter().position(|x| x.key() == key).unwrap_or(0);
            rsx! { TrackCard { list: shelf_tracks, index, class } }
        }
        YtItem::Album(a) => rsx! { YtAlbumCard { album: a, class } },
        YtItem::Artist(a) => rsx! { YtArtistCard { artist: a, class } },
        YtItem::Playlist(p) => rsx! { YtPlaylistCard { playlist: p, class } },
    }
}

// ------------------------------------------------------------------ Spotify

/// Spotify-Playlist; der Play-Knopf lädt sie und spielt die Treffer auf YouTube Music.
/// Nicht abrufbare Playlists (Spotify-App im Entwicklungsmodus) erscheinen gedimmt, ohne Verweis.
#[component]
pub fn SpPlaylistCard(playlist: SpPlaylistSummary, #[props(into, default)] class: String) -> Element {
    let sub = [playlist.owner.clone(), Some(util::tracks_count(playlist.track_count as u64))].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let id = playlist.id.clone();
    if !playlist.readable {
        return rsx! {
            div { class: "block min-w-0 opacity-60 {class}",
                title: tr("Spotify gibt nur Titel eigener und gemeinsamer Playlists frei"),
                div { class: "relative",
                    Cover { src: playlist.image.clone(), class: "aspect-square w-full rounded-lg grayscale", icon: rsx! { icons::ListMusic {} } }
                    div { class: "absolute right-2 bottom-2 grid size-8 place-items-center rounded-full bg-background/80 text-muted-foreground",
                        icons::Lock { class: "size-4 pointer-events-none" }
                    }
                }
                CardText { title: playlist.name.clone(), "{sub}" }
            }
        };
    }
    rsx! {
        Link { class: "group block min-w-0 {class}", to: Route::SpotifyPlaylistPage { id: playlist.id.clone() },
            div { class: "relative",
                Cover { src: playlist.image.clone(), class: "aspect-square w-full rounded-lg", icon: rsx! { icons::ListMusic {} } }
                if playlist.track_count > 0 {
                    HoverPlay {
                        onclick: move |_| {
                            let id = id.clone();
                            spawn_forever(async move {
                                match api::get::<SpPlaylist>(&format!("/spotify/playlists/{}", api::enc(&id))).await {
                                    Ok(p) => crate::spotify::play(p.tracks, 0, false),
                                    Err(e) => crate::spotify::report(e),
                                }
                            });
                        }
                    }
                }
            }
            CardText { title: playlist.name.clone(), "{sub}" }
        }
    }
}
