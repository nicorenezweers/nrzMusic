//! YouTube Music: Suche mit Vorschlägen, Reiter Songs/Videos/Alben/Künstler/Playlists,
//! Regale der Startseite, Album-, Künstler- und Playlist-Seiten. Abgespielt wird die
//! Tonspur, die der Server mit yt-dlp holt (siehe `player::stream_url`).

use crate::api::{self, ApiError};
use crate::components::cards::*;
use crate::components::common::*;
use crate::components::download::DownloadAllButton;
use crate::components::tracks::{Lead, TrackList, pick_playlist};
use crate::i18n::{tr, trf};
use crate::pages::playlists::import_youtube;
use crate::pages::{LoadError, YtOff};
use crate::state;
use crate::{Route, player, util};
use dioxus::prelude::*;
use nrzmusic_types::*;

const TABS: &[&str] = &["songs", "videos", "albums", "artists", "playlists"];

fn tab_label(t: &str) -> String {
    match t {
        "videos" => tr("Videos"),
        "albums" => tr("Alben"),
        "artists" => tr("Künstler"),
        "playlists" => tr("Playlists"),
        _ => tr("Songs"),
    }
}

async fn load_home() -> Result<Vec<YtShelf>, ApiError> {
    let shelves = api::get::<Vec<YtShelf>>("/yt/home").await?;
    Ok(shelves.into_iter().filter(|s| !s.items.is_empty()).collect())
}

fn yt_tracks(list: &[YtTrack]) -> Vec<PlayableTrack> {
    list.iter().cloned().map(PlayableTrack::Youtube).collect()
}

/// Fehler einer YouTube-Seite: ausgeschaltet oder andere Störung.
#[component]
fn YtError(error: ApiError) -> Element {
    if error.youtube_off() {
        rsx! { YtOff {} }
    } else {
        rsx! { LoadError { message: error.message.clone() } }
    }
}

#[component]
pub fn YouTube(q: String, tab: String) -> Element {
    let nav = use_navigator();
    let tab = if TABS.contains(&tab.as_str()) { tab } else { "songs".to_string() };
    let mut text = use_signal(|| q.clone());
    let mut focused = use_signal(|| false);
    let mut highlighted = use_signal(|| None::<usize>);
    let q2 = q.clone();
    use_effect(use_reactive!(|q2| text.set(q2)));

    // Vorschläge beim Tippen (verzögert)
    let suggestions = use_resource(move || async move {
        let t = text();
        if t.trim().len() < 2 || !focused() {
            return Vec::new();
        }
        gloo_timers::future::TimeoutFuture::new(160).await;
        api::get::<Vec<String>>(&format!("/yt/suggest?q={}", api::enc(t.trim()))).await.unwrap_or_default()
    });

    let (qq, tt) = (q.clone(), tab.clone());
    let results = use_resource(use_reactive!(|qq, tt| async move {
        if qq.trim().is_empty() {
            return None;
        }
        Some(api::get::<YtSearchResults>(&format!("/yt/search?q={}&filter={}", api::enc(qq.trim()), tt)).await)
    }));
    let home = use_resource(load_home);

    if !state::youtube_enabled() {
        return rsx! {
            PageHeader { title: tr("YouTube Music") }
            YtOff {}
        };
    }

    let tab_now = tab.clone();
    let go = move |query: String| {
        let (mut f, mut h) = (focused, highlighted);
        f.set(false);
        h.set(None);
        nav.push(Route::YouTube { q: query.trim().to_string(), tab: tab_now.clone() });
    };
    let go2 = go.clone();
    let go3 = go.clone();
    let sugg: Vec<String> = suggestions.read().clone().unwrap_or_default();
    let sugg2 = sugg.clone();
    let open = focused() && !sugg.is_empty();

    rsx! {
        div { class: "mb-6 flex flex-col gap-4",
            div { class: "flex items-center gap-3",
                div { class: "grid size-10 shrink-0 place-items-center rounded-xl bg-muted", icons::Youtube { class: "size-5" } }
                h1 { class: "text-2xl font-semibold tracking-tight md:text-3xl", {tr("YouTube Music")} }
            }
            // Suchfeld mit Vorschlägen
            form { class: "relative max-w-2xl",
                role: "search",
                onsubmit: move |e| {
                    e.prevent_default();
                    go(text());
                },
                icons::Search { class: "pointer-events-none absolute left-3.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" }
                input {
                    class: tw_merge::tw_merge!(INPUT, "h-11 rounded-full pl-10 text-base md:text-base"),
                    r#type: "search",
                    autocomplete: "off",
                    placeholder: tr("Songs, Alben, Künstler, Playlists …"),
                    value: "{text}",
                    onfocus: move |_| focused.set(true),
                    onblur: move |_| {
                        // Klick auf einen Vorschlag zuerst zulassen
                        spawn(async move {
                            gloo_timers::future::TimeoutFuture::new(150).await;
                            focused.set(false);
                        });
                    },
                    oninput: move |e| {
                        text.set(e.value());
                        focused.set(true);
                        highlighted.set(None);
                    },
                    onkeydown: move |e: KeyboardEvent| {
                        let n = sugg2.len();
                        if n == 0 { return; }
                        match e.key() {
                            Key::ArrowDown => {
                                e.prevent_default();
                                highlighted.set(Some(highlighted().map(|i| (i + 1) % n).unwrap_or(0)));
                            }
                            Key::ArrowUp => {
                                e.prevent_default();
                                highlighted.set(Some(highlighted().map(|i| (i + n - 1) % n).unwrap_or(n - 1)));
                            }
                            Key::Enter => {
                                if let Some(i) = highlighted() {
                                    e.prevent_default();
                                    go2(sugg2[i].clone());
                                }
                            }
                            Key::Escape => focused.set(false),
                            _ => {}
                        }
                    },
                }
                if open {
                    div { class: "absolute inset-x-0 top-full z-30 mt-1.5 overflow-hidden rounded-xl border bg-popover p-1 shadow-xl animate-in", role: "listbox",
                        for (i , s) in sugg.iter().cloned().enumerate() {
                            button {
                                key: "{s}",
                                r#type: "button",
                                role: "option",
                                class: if highlighted() == Some(i) { "flex w-full items-center gap-2.5 rounded-lg bg-accent px-3 py-2 text-left text-sm" } else { "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm hover:bg-accent" },
                                onmousedown: move |e| e.prevent_default(),
                                onclick: {
                                    let go = go3.clone();
                                    let s = s.clone();
                                    move |_| go(s.clone())
                                },
                                icons::Search { class: "size-3.5 shrink-0 text-muted-foreground" }
                                span { class: "truncate", "{s}" }
                            }
                        }
                    }
                }
            }
            if !q.trim().is_empty() {
                div { class: "shelf -mx-4 flex gap-2 overflow-x-auto px-4",
                    for t in TABS.iter().copied() {
                        Link {
                            class: if t == tab { "shrink-0 rounded-full bg-primary px-3.5 py-1.5 text-sm font-medium text-primary-foreground" } else { "shrink-0 rounded-full bg-muted px-3.5 py-1.5 text-sm text-muted-foreground hover:text-foreground" },
                            to: Route::YouTube { q: q.clone(), tab: t.to_string() },
                            {tab_label(t)}
                        }
                    }
                }
            }
        }

        if q.trim().is_empty() {
            match &*home.read() {
                None => rsx! { Loading {} },
                Some(Err(e)) => rsx! { YtError { error: e.clone() } },
                Some(Ok(shelves)) if shelves.is_empty() => rsx! {
                    EmptyState { title: tr("Suche nach Musik auf YouTube Music"), description: tr("Titel lassen sich abspielen, in Playlists mischen und favorisieren."), icon: rsx! { icons::Youtube {} } }
                },
                Some(Ok(shelves)) => rsx! {
                    div { class: "flex flex-col gap-9",
                        for (i , shelf) in shelves.iter().cloned().enumerate() {
                            YtShelfRow { key: "{i}-{shelf.title}", shelf }
                        }
                    }
                },
            }
        } else {
            match &*results.read() {
                None | Some(None) => rsx! { Loading {} },
                Some(Some(Err(e))) => rsx! { YtError { error: e.clone() } },
                Some(Some(Ok(r))) => rsx! { YtResults { results: r.clone(), tab: tab.clone() } },
            }
        }
    }
}

#[component]
fn YtShelfRow(shelf: YtShelf) -> Element {
    let tracks: Vec<PlayableTrack> = shelf
        .items
        .iter()
        .filter_map(|i| match i {
            YtItem::Track(t) => Some(PlayableTrack::Youtube(t.clone())),
            _ => None,
        })
        .collect();
    rsx! {
        Shelf { title: shelf.title.clone(),
            for (i , item) in shelf.items.iter().cloned().enumerate() {
                YtItemCard { key: "{i}", item, shelf_tracks: tracks.clone(), class: SHELF_ITEM }
            }
        }
    }
}

#[component]
fn YtResults(results: YtSearchResults, tab: String) -> Element {
    let empty = rsx! { EmptyState { title: tr("Nichts gefunden"), icon: rsx! { icons::Search {} } } };
    match tab.as_str() {
        "videos" | "songs" => {
            let list = if tab == "videos" { &results.videos } else { &results.songs };
            if list.is_empty() {
                return empty;
            }
            let tracks = yt_tracks(list);
            rsx! {
                div { class: "mb-3 flex gap-2", PlayButtons { tracks: tracks.clone() } }
                TrackList { tracks }
            }
        }
        "albums" if !results.albums.is_empty() => rsx! {
            div { class: GRID, for a in results.albums.iter().cloned() { YtAlbumCard { key: "{a.browse_id}", album: a } } }
        },
        "artists" if !results.artists.is_empty() => rsx! {
            div { class: GRID, for a in results.artists.iter().cloned() { YtArtistCard { key: "{a.browse_id}", artist: a } } }
        },
        "playlists" if !results.playlists.is_empty() => rsx! {
            div { class: GRID, for p in results.playlists.iter().cloned() { YtPlaylistCard { key: "{p.playlist_id}", playlist: p } } }
        },
        _ => empty,
    }
}

/// "In nrzMusic importieren" für eine YouTube-Playlist.
#[component]
fn ImportButton(playlist_id: String) -> Element {
    let nav = use_navigator();
    let mut busy = use_signal(|| false);
    rsx! {
        button { class: btn(BTN_OUTLINE), disabled: busy(),
            onclick: move |_| {
                let url = format!("https://music.youtube.com/playlist?list={}", playlist_id);
                spawn(async move {
                    busy.set(true);
                    if let Some(p) = import_youtube(url).await {
                        nav.push(Route::PlaylistPage { id: p.id });
                    }
                    busy.set(false);
                });
            },
            if busy() { icons::LoaderCircle { class: "size-4 animate-spin" } } else { icons::Import { class: "size-4" } }
            {tr("In nrzMusic importieren")}
        }
    }
}

#[component]
fn ExternalButton(url: String) -> Element {
    rsx! {
        a { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", href: "{url}", target: "_blank", rel: "noopener noreferrer", title: tr("Auf YouTube Music öffnen"),
            icons::ExternalLink { class: "size-5" }
        }
    }
}

#[component]
fn YtArtistLine(artists: Vec<YtArtistRef>) -> Element {
    let n = artists.len();
    rsx! {
        for (i , a) in artists.iter().cloned().enumerate() {
            if let Some(id) = a.id.clone() {
                Link { class: "font-medium hover:underline", to: Route::YtArtistPage { id }, "{a.name}" }
            } else {
                span { class: "font-medium", "{a.name}" }
            }
            if i + 2 < n { ", " } else if i + 1 < n { " & " }
        }
    }
}

#[component]
pub fn YtAlbumPage(id: String) -> Element {
    let data = use_resource(use_reactive!(|id| async move { api::get::<YtAlbum>(&format!("/yt/album/{}", api::enc(&id))).await }));
    match &*data.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! { YtError { error: e.clone() } },
        Some(Ok(a)) => {
            let s = a.summary.clone();
            let tracks = yt_tracks(&a.tracks);
            let total: u64 = a.tracks.iter().filter_map(|t| t.duration_ms).sum();
            let mut meta = Vec::new();
            if let Some(y) = &s.year {
                meta.push(y.clone());
            }
            meta.push(util::tracks_count(tracks.len() as u64));
            if total > 0 {
                meta.push(util::long_duration(total));
            }
            let refs: Vec<_> = tracks.iter().map(|t| t.to_ref()).collect();
            let t_queue = tracks.clone();
            rsx! {
                DetailHeader {
                    image: rsx! { Cover { src: s.thumbnail.clone(), class: "aspect-square w-full rounded-xl", eager: true, icon: rsx! { icons::Disc3 {} } } },
                    backdrop: s.thumbnail.clone(),
                    kind: if s.kind.is_empty() { tr("Album") } else { s.kind.clone() },
                    title: s.title.clone(),
                    subtitle: rsx! { span { class: "inline-flex items-center gap-1.5", icons::Youtube { class: "size-4 text-muted-foreground" } YtArtistLine { artists: s.artists.clone() } } },
                    meta: meta.join(" · "),
                    PlayButtons { tracks: tracks.clone() }
                    button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Warteschlange"), onclick: move |_| player::add_to_queue(t_queue.clone()),
                        icons::ListEnd { class: "size-5" }
                    }
                    button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Playlist hinzufügen …"), onclick: move |_| pick_playlist(refs.clone()),
                        icons::ListPlus { class: "size-5" }
                    }
                    DownloadAllButton { tracks: a.tracks.clone(), label: tr("Album herunterladen") }
                    if let Some(pid) = a.playlist_id.clone() {
                        ImportButton { playlist_id: pid }
                    }
                    ExternalButton { url: format!("https://music.youtube.com/browse/{}", s.browse_id) }
                }
                if let Some(d) = a.description.clone().filter(|d| !d.is_empty()) {
                    Description { text: d }
                }
                TrackList { tracks, lead: Lead::Index, show_album: false }
            }
        }
    }
}

#[component]
fn Description(text: String) -> Element {
    let mut open = use_signal(|| false);
    let long = text.len() > 280;
    rsx! {
        div { class: "mb-6 max-w-3xl",
            p { class: if open() || !long { "whitespace-pre-line text-sm text-muted-foreground" } else { "line-clamp-3 whitespace-pre-line text-sm text-muted-foreground" }, "{text}" }
            if long {
                button { class: "mt-1 text-sm font-medium hover:underline", onclick: move |_| open.toggle(),
                    if open() { {tr("Weniger")} } else { {tr("Mehr")} }
                }
            }
        }
    }
}

#[component]
pub fn YtArtistPage(id: String) -> Element {
    let data = use_resource(use_reactive!(|id| async move { api::get::<YtArtist>(&format!("/yt/artist/{}", api::enc(&id))).await }));
    match &*data.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! { YtError { error: e.clone() } },
        Some(Ok(a)) => {
            let s = a.summary.clone();
            let top = yt_tracks(&a.top_songs);
            let first = top.first().cloned();
            rsx! {
                DetailHeader {
                    image: rsx! { Cover { src: s.thumbnail.clone(), round: true, class: "aspect-square w-full", eager: true, icon: rsx! { icons::MicVocal {} } } },
                    backdrop: s.thumbnail.clone(),
                    kind: tr("Künstler · YouTube Music"),
                    title: s.name.clone(),
                    meta: s.subscribers.clone().unwrap_or_default(),
                    PlayButtons { tracks: top.clone() }
                    if let Some(t) = first {
                        button { class: btn(BTN_OUTLINE), onclick: move |_| player::start_radio(t.clone()), icons::Radio {} {tr("Radio starten")} }
                    }
                    if let Some(pid) = a.songs_playlist_id.clone() {
                        Link { class: "{btn(BTN_GHOST)} rounded-full", to: Route::YtPlaylistPage { id: pid }, icons::ListMusic {} {tr("Alle Songs")} }
                    }
                    ExternalButton { url: format!("https://music.youtube.com/channel/{}", s.browse_id) }
                }
                if let Some(d) = a.description.clone().filter(|d| !d.is_empty()) {
                    Description { text: d }
                }
                div { class: "flex flex-col gap-10",
                    if !top.is_empty() {
                        section {
                            h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Top-Songs")} }
                            TrackList { tracks: top }
                        }
                    }
                    if !a.albums.is_empty() {
                        section {
                            h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Alben")} }
                            div { class: GRID, for al in a.albums.iter().cloned() { YtAlbumCard { key: "{al.browse_id}", album: al } } }
                        }
                    }
                    if !a.singles.is_empty() {
                        section {
                            h2 { class: "mb-3 text-lg font-semibold tracking-tight", {tr("Singles & EPs")} }
                            div { class: GRID, for al in a.singles.iter().cloned() { YtAlbumCard { key: "{al.browse_id}", album: al } } }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn YtPlaylistPage(id: String) -> Element {
    let pid = id.clone();
    let data = use_resource(use_reactive!(|pid| async move { api::get::<YtPlaylist>(&format!("/yt/playlist/{}", api::enc(&pid))).await }));
    match &*data.read() {
        None => rsx! { Loading {} },
        Some(Err(e)) => rsx! { YtError { error: e.clone() } },
        Some(Ok(p)) => {
            let s = p.summary.clone();
            let tracks = yt_tracks(&p.tracks);
            let total: u64 = p.tracks.iter().filter_map(|t| t.duration_ms).sum();
            let mut meta = Vec::new();
            if let Some(a) = &s.author {
                meta.push(a.clone());
            }
            meta.push(s.track_count.clone().unwrap_or_else(|| util::tracks_count(tracks.len() as u64)));
            if total > 0 {
                meta.push(util::long_duration(total));
            }
            let refs: Vec<_> = tracks.iter().map(|t| t.to_ref()).collect();
            let t_queue = tracks.clone();
            rsx! {
                DetailHeader {
                    image: rsx! { Cover { src: s.thumbnail.clone(), class: "aspect-square w-full rounded-xl", eager: true, icon: rsx! { icons::ListMusic {} } } },
                    backdrop: s.thumbnail.clone(),
                    kind: tr("Playlist · YouTube Music"),
                    title: s.title.clone(),
                    meta: meta.join(" · "),
                    PlayButtons { tracks: tracks.clone() }
                    button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Warteschlange"), onclick: move |_| player::add_to_queue(t_queue.clone()),
                        icons::ListEnd { class: "size-5" }
                    }
                    button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Playlist hinzufügen …"), onclick: move |_| pick_playlist(refs.clone()),
                        icons::ListPlus { class: "size-5" }
                    }
                    DownloadAllButton { tracks: p.tracks.clone(), label: tr("Alle herunterladen") }
                    ImportButton { playlist_id: id.clone() }
                    ExternalButton { url: format!("https://music.youtube.com/playlist?list={id}") }
                }
                if let Some(d) = p.description.clone().filter(|d| !d.is_empty()) {
                    Description { text: d }
                }
                if tracks.is_empty() {
                    EmptyState { title: tr("Diese Playlist ist leer"), icon: rsx! { icons::ListMusic {} } }
                } else {
                    TrackList { tracks, lead: Lead::Index }
                }
                p { class: "mt-6 text-xs text-muted-foreground", {trf("Quelle: YouTube Music · {0}", &[&s.playlist_id])} }
            }
        }
    }
}
