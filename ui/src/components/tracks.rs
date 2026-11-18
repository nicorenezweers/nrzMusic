//! Titellisten mit Kontextmenü (Rechtsklick oder "…"), Doppelklick/Enter spielt ab der Zeile,
//! "Zur Playlist hinzufügen …" und "Neue Playlist" als Dialoge, die einmal in der App liegen.

use crate::components::common::*;
use crate::components::download::DownloadBadge;
use crate::components::overlay::*;
use crate::downloads::{self, DlState};
use crate::i18n::{tr, trf};
use crate::player;
use crate::state::{self, PLAYLISTS};
use crate::util;
use crate::{Route, api};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::{PlayableTrack, PlaylistSummary, TrackRef};

// ------------------------------------------------------------------ Verweise

/// Album eines Titels (lokal oder YouTube).
pub fn album_route(t: &PlayableTrack) -> Option<Route> {
    match t {
        PlayableTrack::Local(l) if !l.album_id.is_empty() && !l.album.is_empty() => Some(Route::AlbumPage { id: l.album_id.clone() }),
        PlayableTrack::Youtube(y) => y.album.as_ref().and_then(|a| a.id.clone()).map(|id| Route::YtAlbumPage { id }),
        _ => None,
    }
}

/// (Erster) Künstler eines Titels.
pub fn artist_route(t: &PlayableTrack) -> Option<Route> {
    match t {
        PlayableTrack::Local(l) if !l.artist_id.is_empty() => Some(Route::ArtistPage { id: l.artist_id.clone() }),
        PlayableTrack::Youtube(y) => y.artists.iter().find_map(|a| a.id.clone()).map(|id| Route::YtArtistPage { id }),
        _ => None,
    }
}

/// Künstlerzeile mit Verweis(en).
#[component]
pub fn ArtistLinks(track: PlayableTrack, #[props(into, default)] class: String) -> Element {
    match &track {
        PlayableTrack::Youtube(y) if y.artists.len() > 1 => {
            let n = y.artists.len();
            rsx! {
                span { class: "truncate {class}",
                    for (i , a) in y.artists.iter().cloned().enumerate() {
                        if let Some(id) = a.id.clone() {
                            Link { class: "hover:underline", to: Route::YtArtistPage { id }, onclick: move |e: MouseEvent| e.stop_propagation(), "{a.name}" }
                        } else {
                            span { "{a.name}" }
                        }
                        if i + 2 < n { ", " } else if i + 1 < n { " & " }
                    }
                }
            }
        }
        _ => {
            let name = track.artist();
            match artist_route(&track) {
                Some(r) => rsx! {
                    Link { class: "truncate hover:underline {class}", to: r, onclick: move |e: MouseEvent| e.stop_propagation(), "{name}" }
                },
                None => rsx! { span { class: "truncate {class}", "{name}" } },
            }
        }
    }
}

// ------------------------------------------------------------------ Liste

/// Zusätzliche Einträge im Kontextmenü, je nach Liste.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuExtra {
    RemoveFromPlaylist { playlist: i64, position: usize },
    RemoveFromQueue(u64),
}

#[derive(Clone)]
pub struct MenuState {
    x: f64,
    y: f64,
    track: PlayableTrack,
    extra: Vec<MenuExtra>,
}

static MENU: GlobalSignal<Option<MenuState>> = Signal::global(|| None);

/// Kontextmenü für einen Titel an einer Bildschirmposition öffnen.
pub fn open_menu(x: f64, y: f64, track: PlayableTrack, extra: Vec<MenuExtra>) {
    *MENU.write() = Some(MenuState { x, y, track, extra });
}

/// Kontextmenü schliessen; `true`, wenn eines offen war.
pub fn close_menu() -> bool {
    let open = MENU.peek().is_some();
    if open {
        *MENU.write() = None;
    }
    open || crate::pages::spotify::close_menu()
}

/// Bildschirmposition eines Maus-Ereignisses.
pub fn client_xy(e: &MouseEvent) -> (f64, f64) {
    let p = e.client_coordinates();
    (p.x, p.y)
}

/// Kontextmenü-Knopf ("…") für einen Titel.
#[component]
pub fn MenuButton(track: PlayableTrack, #[props(default)] extra: Vec<MenuExtra>, #[props(into, default)] class: String) -> Element {
    rsx! {
        button {
            class: "{ICON_BTN} {class}",
            title: tr("Mehr"),
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                let (x, y) = client_xy(&e);
                open_menu(x, y, track.clone(), extra.clone());
            },
            icons::Ellipsis { class: "size-4" }
        }
    }
}

/// Wird auf Touch-Geräten benutzt (dort spielt ein einfacher Tipp).
pub fn coarse_pointer() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(pointer: coarse)").ok().flatten())
        .is_some_and(|m| m.matches())
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Lead {
    /// Kleines Cover (96 px)
    #[default]
    Cover,
    /// Titelnummer (Album)
    Number,
    /// Laufende Nummer (Playlist, Verlauf)
    Index,
}

/// Titelliste. Doppelklick/Enter (Touch: Tipp) spielt die ganze Liste ab dieser Zeile.
#[component]
pub fn TrackList(
    tracks: Vec<PlayableTrack>,
    #[props(default)] lead: Lead,
    #[props(default = true)] show_album: bool,
    /// Playlist-ID: Einträge entfernbar und per Ziehen umstellbar
    #[props(optional)] playlist: Option<i64>,
    #[props(optional)] on_move: Option<EventHandler<(usize, usize)>>,
    /// Weitere Spalte (z. B. Wiedergaben, Datum)
    #[props(optional)] extra: Option<Vec<String>>,
) -> Element {
    let current = player::current_key();
    let playing = player::PLAYING();
    let mut drag_from = use_signal(|| None::<usize>);
    let mut drag_over = use_signal(|| None::<usize>);
    let reorder = on_move.is_some();
    let multi_disc = matches!(lead, Lead::Number)
        && tracks.iter().filter_map(|t| match t { PlayableTrack::Local(l) => l.disc_no, _ => None }).max().unwrap_or(1) > 1;

    rsx! {
        div { class: "flex flex-col", role: "list",
            for (i , t) in tracks.iter().cloned().enumerate() {
                {
                    let key = t.key();
                    let is_current = current.as_deref() == Some(key.as_str());
                    let all = tracks.clone();
                    let all2 = tracks.clone();
                    let all3 = tracks.clone();
                    let t_menu = t.clone();
                    let extra_items: Vec<MenuExtra> = playlist.map(|p| vec![MenuExtra::RemoveFromPlaylist { playlist: p, position: i }]).unwrap_or_default();
                    let extra_items2 = extra_items.clone();
                    let disc_header = if multi_disc {
                        let disc = |x: &PlayableTrack| match x { PlayableTrack::Local(l) => l.disc_no.unwrap_or(1), _ => 1 };
                        let d = disc(&t);
                        (i == 0 || disc(&tracks[i - 1]) != d).then_some(d)
                    } else {
                        None
                    };
                    let number = match (&lead, &t) {
                        (Lead::Number, PlayableTrack::Local(l)) => l.track_no.map(|n| n.to_string()).unwrap_or_else(|| (i + 1).to_string()),
                        _ => (i + 1).to_string(),
                    };
                    let over = drag_over() == Some(i) && drag_from().is_some_and(|f| f != i);
                    let extra_col = extra.as_ref().and_then(|e| e.get(i).cloned());
                    rsx! {
                        div { key: "{i}-{key}",
                        if let Some(d) = disc_header {
                            div { class: "mt-4 mb-1 flex items-center gap-2 px-3 text-xs font-medium uppercase tracking-wider text-muted-foreground first:mt-0",
                                icons::Disc { class: "size-3.5" }
                                {trf("CD {0}", &[&d])}
                            }
                        }
                        div {
                            role: "listitem",
                            tabindex: "0",
                            draggable: if reorder { "true" } else { "false" },
                            class: format!(
                                "group grid cursor-default select-none items-center gap-3 rounded-lg px-2 py-1.5 outline-none transition-colors hover:bg-accent/50 focus-visible:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring/50 {} {} {}",
                                if show_album { "grid-cols-[2.5rem_minmax(0,1fr)_auto] md:grid-cols-[2.5rem_minmax(0,3fr)_minmax(0,2fr)_auto]" } else { "grid-cols-[2.5rem_minmax(0,1fr)_auto]" },
                                if is_current { "bg-accent/40" } else { "" },
                                if over { "ring-2 ring-foreground/40" } else { "" },
                            ),
                            onclick: move |_| {
                                if coarse_pointer() {
                                    player::play_now(all.clone(), i);
                                }
                            },
                            ondoubleclick: move |_| player::play_now(all2.clone(), i),
                            onkeydown: move |e: KeyboardEvent| {
                                if e.key() == Key::Enter {
                                    player::play_now(all3.clone(), i);
                                }
                            },
                            oncontextmenu: move |e: MouseEvent| {
                                e.prevent_default();
                                let (x, y) = client_xy(&e);
                                open_menu(x, y, t_menu.clone(), extra_items.clone());
                            },
                            ondragstart: move |e: DragEvent| {
                                if !reorder {
                                    return;
                                }
                                if let Some(dt) = e.data().downcast::<web_sys::DragEvent>().and_then(|d| d.data_transfer()) {
                                    let _ = dt.set_data("text/plain", &i.to_string());
                                    dt.set_effect_allowed("move");
                                }
                                drag_from.set(Some(i));
                            },
                            ondragover: move |e: DragEvent| {
                                if drag_from().is_some() {
                                    e.prevent_default();
                                    if drag_over() != Some(i) {
                                        drag_over.set(Some(i));
                                    }
                                }
                            },
                            ondrop: move |e: DragEvent| {
                                if let Some(from) = drag_from() {
                                    e.prevent_default();
                                    e.stop_propagation();
                                    if from != i
                                        && let Some(h) = on_move {
                                            h.call((from, i));
                                        }
                                }
                                drag_from.set(None);
                                drag_over.set(None);
                            },
                            ondragend: move |_| {
                                drag_from.set(None);
                                drag_over.set(None);
                            },

                            // Vorne: Cover oder Nummer, beim Darüberfahren ein Play-Knopf
                            div { class: "relative grid size-10 place-items-center",
                                match lead {
                                    Lead::Cover => rsx! { Cover { src: t.artwork_url(96), class: "size-10" } },
                                    _ => rsx! {
                                        if is_current {
                                            EqBars { playing }
                                        } else {
                                            span { class: "text-sm tabular-nums text-muted-foreground", "{number}" }
                                        }
                                    },
                                }
                                button {
                                    class: "absolute inset-0 grid place-items-center rounded-md bg-black/50 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100",
                                    title: tr("Abspielen"),
                                    onclick: {
                                        let all = tracks.clone();
                                        move |e: MouseEvent| {
                                            e.stop_propagation();
                                            if is_current {
                                                player::toggle();
                                            } else {
                                                player::play_now(all.clone(), i);
                                            }
                                        }
                                    },
                                    if is_current && playing {
                                        icons::Pause { class: "size-4 fill-current pointer-events-none" }
                                    } else {
                                        icons::Play { class: "size-4 fill-current pointer-events-none" }
                                    }
                                }
                            }
                            // Titel und Künstler
                            div { class: "flex min-w-0 flex-col",
                                div { class: "flex min-w-0 items-center gap-1.5",
                                    span { class: if is_current { "truncate text-sm font-semibold" } else { "truncate text-sm font-medium" }, "{t.title()}" }
                                    if let PlayableTrack::Youtube(y) = &t {
                                        if y.explicit { Tag { text: "E" } }
                                        if y.is_video { Tag { text: tr("Video") } }
                                    }
                                }
                                div { class: "flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground",
                                    if let PlayableTrack::Youtube(y) = &t {
                                        icons::Youtube { class: "size-3.5 shrink-0" }
                                        DownloadBadge { video_id: y.video_id.clone() }
                                    }
                                    ArtistLinks { track: t.clone() }
                                }
                            }
                            // Album
                            if show_album {
                                div { class: "hidden min-w-0 text-sm text-muted-foreground md:block",
                                    if let (Some(name), Some(r)) = (t.album().map(String::from), album_route(&t)) {
                                        Link { class: "block truncate hover:text-foreground hover:underline", to: r, onclick: move |e: MouseEvent| e.stop_propagation(), "{name}" }
                                    } else if let Some(name) = t.album() {
                                        span { class: "block truncate", "{name}" }
                                    }
                                }
                            }
                            // Rechts: Extra, Herz, Dauer, Menü
                            div { class: "flex items-center gap-1",
                                if let Some(x) = extra_col {
                                    span { class: "hidden w-24 truncate text-right text-xs text-muted-foreground lg:block", "{x}" }
                                }
                                Heart { track: t.clone(), class: "opacity-100 md:opacity-0 md:group-hover:opacity-100 md:focus-visible:opacity-100 [&.text-heart]:opacity-100" }
                                span { class: "hidden w-12 text-right text-xs tabular-nums text-muted-foreground sm:block",
                                    {t.duration_ms().map(util::clock_ms).unwrap_or_default()}
                                }
                                MenuButton { track: t.clone(), extra: extra_items2 }
                            }
                        }
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Kontextmenü

#[component]
pub fn TrackMenuHost() -> Element {
    let Some(m) = MENU.read().clone() else { return rsx! {} };
    let nav = use_navigator();
    let (vw, vh) = web_sys::window()
        .map(|w| (w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1024.0), w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(768.0)))
        .unwrap_or((1024.0, 768.0));
    const W: f64 = 248.0;
    const H: f64 = 420.0;
    let left = if m.x + W + 8.0 > vw { (m.x - W).max(8.0) } else { m.x };
    let top = if m.y + H + 8.0 > vh { (vh - H - 8.0).max(8.0) } else { m.y };
    let close = move || *MENU.write() = None;
    let t = m.track.clone();
    let is_yt = matches!(t, PlayableTrack::Youtube(_));
    let album = album_route(&t);
    let artist = artist_route(&t);
    let fav = state::is_favorite(&t.key());
    let (t1, t2, t3, t4, t5) = (t.clone(), t.clone(), t.clone(), t.clone(), t.clone());
    rsx! {
        div { class: "fixed inset-0 z-[65]", onclick: move |_| close(), oncontextmenu: move |e| { e.prevent_default(); close(); } }
        div {
            class: "fixed z-[66] w-[15.5rem] overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl animate-in",
            style: "left: {left}px; top: {top}px",
            role: "menu",
            onkeydown: move |e| if e.key() == Key::Escape { close() },
            div { class: "truncate px-2 pt-1 pb-1.5 text-xs font-medium text-muted-foreground", "{t.title()}" }
            Item { onclick: move |_| { close(); player::play_now(vec![t1.clone()], 0); }, icons::Play {} {tr("Abspielen")} }
            Item { onclick: move |_| { close(); player::play_next(vec![t2.clone()]); }, icons::ListStart {} {tr("Als Nächstes")} }
            Item { onclick: move |_| { close(); player::add_to_queue(vec![t3.clone()]); }, icons::ListEnd {} {tr("Zur Warteschlange")} }
            Item { onclick: move |_| { close(); pick_playlist(vec![t4.to_ref()]); }, icons::ListPlus {} {tr("Zur Playlist hinzufügen …")} }
            Item { onclick: move |_| { close(); state::toggle_favorite(t5.clone()); },
                icons::Heart {}
                if fav { {tr("Aus den Favoriten entfernen")} } else { {tr("Favorit")} }
            }
            if is_yt {
                Item { onclick: { let t = t.clone(); move |_| { close(); player::start_radio(t.clone()); } }, icons::Radio {} {tr("Radio starten")} }
            }
            Item { onclick: { let t = t.clone(); move |_| { close(); crate::share::share_track(&t); } }, icons::Share2 {} {tr("Teilen …")} }
            if crate::android::available() {
                if crate::android::is_offline(&t.key()) {
                    Item { onclick: { let k = t.key(); move |_| { close(); crate::android::remove(&k); } }, icons::CircleCheck {} {tr("Vom Handy entfernen")} }
                } else {
                    Item { onclick: { let t = t.clone(); move |_| { close(); crate::android::save(vec![t.clone()]); } }, icons::Smartphone {} {tr("Aufs Handy laden (offline)")} }
                }
            }
            div { class: "-mx-1 my-1 h-px bg-border" }
            if let Some(r) = album {
                Item { onclick: move |_| { close(); nav.push(r.clone()); }, icons::Disc3 {} {tr("Zum Album")} }
            }
            if let Some(r) = artist {
                Item { onclick: move |_| { close(); nav.push(r.clone()); }, icons::MicVocal {} {tr("Zum Künstler")} }
            }
            match &t {
                PlayableTrack::Local(l) => rsx! {
                    a { class: ITEM, role: "menuitem", href: "/api/v1/tracks/{l.id}/download", download: "", onclick: move |_| close(),
                        icons::Download {} {tr("Herunterladen")}
                    }
                },
                PlayableTrack::Youtube(y) => {
                    let yt = y.clone();
                    rsx! {
                        match downloads::state_of(&y.video_id) {
                            Some(DlState::Downloaded(id)) => rsx! {
                                a { class: ITEM, role: "menuitem", href: "/api/v1/tracks/{id}/download", download: "", title: tr("Auf dem Server"), onclick: move |_| close(),
                                    icons::HardDriveDownload {} {tr("Datei herunterladen")}
                                }
                            },
                            Some(DlState::Queued(id) | DlState::Running(id, _)) => rsx! {
                                Item { onclick: move |_| { close(); downloads::remove(id); }, icons::CircleX {} {tr("Download abbrechen")} }
                            },
                            Some(DlState::Failed(id, _)) => rsx! {
                                Item { onclick: move |_| { close(); downloads::retry(id); }, icons::RotateCw {} {tr("Erneut herunterladen")} }
                            },
                            None if downloads::available() => rsx! {
                                Item { onclick: move |_| { close(); downloads::download(vec![yt.clone()]); }, icons::Download {} {tr("Herunterladen")} }
                            },
                            None => rsx! {},
                        }
                        a { class: ITEM, role: "menuitem", href: util::yt_music_url(&y.video_id), target: "_blank", rel: "noopener noreferrer", onclick: move |_| close(),
                            icons::ExternalLink {} {tr("Auf YouTube Music öffnen")}
                        }
                    }
                }
            }
            for x in m.extra.iter().cloned() {
                div { class: "-mx-1 my-1 h-px bg-border" }
                match x {
                    MenuExtra::RemoveFromPlaylist { playlist, position } => rsx! {
                        Item { destructive: true,
                            onclick: move |_| {
                                close();
                                spawn_forever(async move {
                                    match api::delete_json::<PlaylistSummary>(&format!("/playlists/{playlist}/items/{position}")).await {
                                        Ok(_) => { toast(tr("Aus der Playlist entfernt.")); state::bump_playlists(); }
                                        Err(e) => toast_error(e),
                                    }
                                });
                            },
                            icons::Trash2 {} {tr("Aus der Playlist entfernen")}
                        }
                    },
                    MenuExtra::RemoveFromQueue(uid) => rsx! {
                        Item { destructive: true, onclick: move |_| { close(); player::remove(uid); }, icons::ListX {} {tr("Aus der Warteschlange entfernen")} }
                    },
                }
            }
        }
    }
}

pub const ITEM: &str = "relative flex w-full cursor-default select-none items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-accent hover:text-accent-foreground focus-visible:bg-accent [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:pointer-events-none";

#[component]
pub fn Item(onclick: EventHandler<MouseEvent>, #[props(default)] destructive: bool, children: Element) -> Element {
    let c = if destructive { "text-destructive hover:text-destructive hover:bg-destructive/10" } else { "" };
    rsx! {
        button { class: "{ITEM} {c}", role: "menuitem", onclick: move |e| onclick.call(e), {children} }
    }
}

// ------------------------------------------------------------------ Playlist wählen / anlegen

static PICK: GlobalSignal<Vec<TrackRef>> = Signal::global(Vec::new);
static PICK_OPEN: GlobalSignal<bool> = Signal::global(|| false);
static NEW_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// "Zur Playlist hinzufügen …" für diese Titel öffnen.
pub fn pick_playlist(items: Vec<TrackRef>) {
    if items.is_empty() {
        return;
    }
    *PICK.write() = items;
    *PICK_OPEN.write() = true;
}

/// Dialog "Neue Playlist" öffnen.
pub fn new_playlist() {
    *NEW_OPEN.write() = true;
}

#[component]
pub fn PlaylistPicker() -> Element {
    let mut name = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let open = PICK_OPEN.signal();
    let items = PICK.read().clone();
    let n = items.len();
    let items2 = items.clone();
    let create = move || {
        let items = items2.clone();
        let n = name().trim().to_string();
        if n.is_empty() || busy() {
            return;
        }
        spawn_forever(async move {
            busy.set(true);
            if state::create_playlist(n, String::new(), items).await.is_some() {
                name.set(String::new());
                *PICK_OPEN.write() = false;
            }
            busy.set(false);
        });
    };
    rsx! {
        Modal { open, title: tr("Zur Playlist hinzufügen"), description: if n == 1 { tr("1 Titel") } else { trf("{0} Titel", &[&n]) },
            div { class: "grid gap-4",
                form { class: "flex gap-2",
                    onsubmit: move |e| { e.prevent_default(); create(); },
                    input { class: INPUT, placeholder: tr("Neue Playlist …"), value: "{name}", oninput: move |e| name.set(e.value()) }
                    button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: busy() || name().trim().is_empty(),
                        icons::Plus {} {tr("Anlegen")}
                    }
                }
                div { class: "-mx-2 flex max-h-80 flex-col overflow-y-auto",
                    if PLAYLISTS.read().is_empty() {
                        p { class: "px-2 py-6 text-center text-sm text-muted-foreground", {tr("Noch keine Playlists.")} }
                    }
                    for p in PLAYLISTS.read().iter().cloned() {
                        button {
                            key: "{p.id}",
                            class: "flex items-center gap-3 rounded-lg px-2 py-1.5 text-left hover:bg-accent",
                            onclick: {
                                let items = items.clone();
                                move |_| {
                                    let (id, pname, items) = (p.id, p.name.clone(), items.clone());
                                    *PICK_OPEN.write() = false;
                                    spawn_forever(async move { state::add_to_playlist(id, &pname, items).await; });
                                }
                            },
                            Collage { artwork: p.artwork.clone(), class: "size-10" }
                            span { class: "min-w-0 flex-1",
                                span { class: "block truncate text-sm font-medium", "{p.name}" }
                                span { class: "block text-xs text-muted-foreground", {util::tracks_count(p.item_count as u64)} }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn NewPlaylistDialog() -> Element {
    let nav = use_navigator();
    let mut name = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let open = NEW_OPEN.signal();
    let save = move || {
        let n = name().trim().to_string();
        if n.is_empty() || busy() {
            return;
        }
        spawn_forever(async move {
            busy.set(true);
            if let Some(p) = state::create_playlist(n, description().trim().to_string(), Vec::new()).await {
                name.set(String::new());
                description.set(String::new());
                *NEW_OPEN.write() = false;
                nav.push(Route::PlaylistPage { id: p.id });
            }
            busy.set(false);
        });
    };
    rsx! {
        Modal { open, title: tr("Neue Playlist"),
            form { class: "grid gap-4",
                onsubmit: move |e| { e.prevent_default(); save(); },
                Field { label: tr("Name"),
                    input { class: INPUT, value: "{name}", placeholder: tr("z. B. Sommerabend"), oninput: move |e| name.set(e.value()) }
                }
                Field { label: tr("Beschreibung"),
                    textarea { class: TEXTAREA, rows: "3", value: "{description}", oninput: move |e| description.set(e.value()) }
                }
                div { class: "flex justify-end gap-2 pt-1",
                    button { r#type: "button", class: btn(BTN_OUTLINE), onclick: move |_| *NEW_OPEN.write() = false, {tr("Abbrechen")} }
                    button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: busy() || name().trim().is_empty(), {tr("Anlegen")} }
                }
            }
        }
    }
}
