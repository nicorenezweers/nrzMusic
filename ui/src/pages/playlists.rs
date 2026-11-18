//! Playlists: Übersicht (anlegen, von YouTube importieren) und Detailseite (umbenennen,
//! Beschreibung, löschen, Einträge entfernen und umstellen, abspielen).

use crate::api;
use crate::components::cards::*;
use crate::components::common::*;
use crate::components::download::DownloadYoutubeItem;
use crate::components::overlay::*;
use crate::components::tracks::{Lead, TrackList, new_playlist, pick_playlist};
use crate::i18n::{tr, trf};
use crate::pages::{LibraryTabs, LoadError};
use crate::state::{self, LIBRARY_VERSION, PLAYLISTS, PLAYLISTS_VERSION};
use crate::{Route, player, util};
use dioxus::prelude::*;
use nrzmusic_types::{ImportRequest, Playlist, PlaylistSummary, PlaylistUpdate, Reorder};

#[component]
pub fn Playlists() -> Element {
    let import_open = use_signal(|| false);
    // Beim Öffnen frisch laden
    use_future(state::load_playlists);
    let list = PLAYLISTS.read().clone();
    let mut import = import_open;
    rsx! {
        LibraryTabs {}
        PageHeader { title: tr("Playlists"),
            description: if list.is_empty() { String::new() } else { trf("{0} Playlists", &[&list.len()]) },
            actions: rsx! {
                if state::youtube_enabled() {
                    button { class: btn(BTN_OUTLINE), onclick: move |_| import.set(true), icons::Import {} {tr("Von YouTube importieren")} }
                }
                button { class: btn(BTN_PRIMARY), onclick: move |_| new_playlist(), icons::Plus {} {tr("Neue Playlist")} }
            }
        }
        if list.is_empty() {
            EmptyState { title: tr("Noch keine Playlists"), description: tr("Lege eine an oder speichere die Warteschlange als Playlist."), icon: rsx! { icons::ListMusic {} },
                button { class: btn(BTN_OUTLINE), onclick: move |_| new_playlist(), icons::Plus {} {tr("Neue Playlist")} }
            }
        } else {
            div { class: GRID,
                for p in list.iter().cloned() {
                    PlaylistCard { key: "{p.id}", playlist: p }
                }
            }
        }
        ImportDialog { open: import_open }
    }
}

/// YouTube-(Music-)Playlist per Link als nrzMusic-Playlist übernehmen.
pub async fn import_youtube(url: String) -> Option<PlaylistSummary> {
    match api::post::<PlaylistSummary, _>("/yt/import", &ImportRequest { url }).await {
        Ok(p) => {
            toast(trf("„{0}“ importiert ({1}).", &[&p.name, &util::tracks_count(p.item_count as u64)]));
            state::bump_playlists();
            Some(p)
        }
        Err(e) => {
            toast_error(e);
            None
        }
    }
}

#[component]
fn ImportDialog(open: Signal<bool>) -> Element {
    let nav = use_navigator();
    let mut url = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut open2 = open;
    rsx! {
        Modal { open, title: tr("Von YouTube Music importieren"), description: tr("Link einer Playlist von YouTube oder YouTube Music einfügen."),
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    let u = url().trim().to_string();
                    if u.is_empty() || busy() { return; }
                    spawn(async move {
                        busy.set(true);
                        if let Some(p) = import_youtube(u).await {
                            url.set(String::new());
                            open2.set(false);
                            nav.push(Route::PlaylistPage { id: p.id });
                        }
                        busy.set(false);
                    });
                },
                Field { label: tr("Link"),
                    input { class: INPUT, r#type: "url", placeholder: "https://music.youtube.com/playlist?list=…", value: "{url}", oninput: move |e| url.set(e.value()) }
                }
                div { class: "flex justify-end gap-2",
                    button { r#type: "button", class: btn(BTN_OUTLINE), onclick: move |_| open2.set(false), {tr("Abbrechen")} }
                    button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: busy() || url().trim().is_empty(),
                        if busy() { icons::LoaderCircle { class: "size-4 animate-spin" } } else { icons::Import { class: "size-4" } }
                        {tr("Importieren")}
                    }
                }
            }
        }
    }
}

#[component]
pub fn PlaylistPage(id: i64) -> Element {
    let nav = use_navigator();
    let mut edit_open = use_signal(|| false);
    // Lokale Kopie, damit Umstellen sofort sichtbar ist
    let mut local = use_signal(|| None::<Playlist>);
    let data = use_resource(use_reactive!(|id| async move {
        let _ = (PLAYLISTS_VERSION(), LIBRARY_VERSION());
        api::get::<Playlist>(&format!("/playlists/{id}")).await
    }));
    use_effect(move || {
        if let Some(Ok(p)) = &*data.read() {
            local.set(Some(p.clone()));
        }
    });
    let state_now = data.read().clone();
    match state_now {
        None => return rsx! { Loading {} },
        Some(Err(e)) => return rsx! { LoadError { message: e.to_string() } },
        Some(Ok(_)) => {}
    }
    let Some(p) = local().filter(|p| p.summary.id == id) else { return rsx! { Loading {} } };
    let s = p.summary.clone();
    let tracks = p.items.clone();
    let mut meta = vec![util::tracks_count(s.item_count as u64)];
    if s.duration_ms > 0 {
        meta.push(util::long_duration(s.duration_ms));
    }
    let refs: Vec<_> = tracks.iter().map(|t| t.to_ref()).collect();
    let t_queue = tracks.clone();
    let name = s.name.clone();
    let on_move = move |(from, to): (usize, usize)| {
        let Some(mut p) = local() else { return };
        let n = p.items.len();
        if from >= n || to >= n {
            return;
        }
        let mut order: Vec<u32> = (0..n as u32).collect();
        let x = order.remove(from);
        order.insert(to, x);
        let it = p.items.remove(from);
        p.items.insert(to, it);
        local.set(Some(p));
        spawn(async move {
            match api::put::<PlaylistSummary, _>(&format!("/playlists/{id}/order"), &Reorder { order }).await {
                Ok(_) => state::bump_playlists(),
                Err(e) => {
                    toast_error(e);
                    state::bump_playlists();
                }
            }
        });
    };
    rsx! {
        DetailHeader {
            image: rsx! { Collage { artwork: s.artwork.clone(), class: "aspect-square w-full rounded-xl" } },
            backdrop: s.artwork.first().cloned(),
            kind: if s.youtube_playlist_id.is_some() { tr("Playlist · aus YouTube Music") } else { tr("Playlist") },
            title: s.name.clone(),
            subtitle: rsx! {
                if !s.description.is_empty() {
                    p { class: "max-w-2xl whitespace-pre-line text-muted-foreground", "{s.description}" }
                }
            },
            meta: meta.join(" · "),
            PlayButtons { tracks: tracks.clone() }
            button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Zur Warteschlange"), disabled: tracks.is_empty(), onclick: move |_| player::add_to_queue(t_queue.clone()),
                icons::ListEnd { class: "size-5" }
            }
            button { class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full", title: tr("Playlist teilen …"),
                onclick: { let n = s.name.clone(); move |_| crate::share::share_playlist(id, &n) },
                icons::Share2 { class: "size-5" }
            }
            Dropdown { title: tr("Mehr"),
                trigger_class: "{btn(BTN_GHOST)} {BTN_ICON} rounded-full",
                trigger: rsx! { icons::Ellipsis { class: "size-5" } },
                MenuItem { onclick: move |_| edit_open.set(true), icons::Pencil {} {tr("Umbenennen & Beschreibung")} }
                MenuItem { disabled: refs.is_empty(), onclick: move |_| pick_playlist(refs.clone()), icons::ListPlus {} {tr("Zu anderer Playlist hinzufügen …")} }
                DownloadYoutubeItem { tracks: tracks.clone() }
                if let Some(yid) = s.youtube_playlist_id.clone() {
                    MenuItem { onclick: move |_| util::open_external(&format!("https://music.youtube.com/playlist?list={yid}")), icons::ExternalLink {} {tr("Auf YouTube Music öffnen")} }
                }
                MenuSeparator {}
                MenuItem { destructive: true,
                    onclick: move |_| {
                        let name = name.clone();
                        spawn(async move {
                            let ask = Ask::new(trf("Playlist „{0}“ löschen?", &[&name]), tr("Die Titel selbst bleiben in der Bibliothek.")).confirm(tr("Löschen")).destructive();
                            if confirm(ask).await {
                                match api::delete(&format!("/playlists/{id}")).await {
                                    Ok(()) => {
                                        toast(tr("Playlist gelöscht."));
                                        state::bump_playlists();
                                        nav.replace(Route::Playlists {});
                                    }
                                    Err(e) => toast_error(e),
                                }
                            }
                        });
                    },
                    icons::Trash2 {} {tr("Löschen")}
                }
            }
        }
        if tracks.is_empty() {
            EmptyState { title: tr("Diese Playlist ist leer"), description: tr("Titel über das Menü „…“ oder per Rechtsklick hinzufügen."), icon: rsx! { icons::ListMusic {} } }
        } else {
            p { class: "mb-2 hidden text-xs text-muted-foreground md:block", {tr("Zum Umstellen Zeilen ziehen.")} }
            TrackList { tracks, lead: Lead::Index, playlist: id, on_move }
        }
        EditDialog { open: edit_open, summary: s.clone() }
    }
}

#[component]
fn EditDialog(open: Signal<bool>, summary: PlaylistSummary) -> Element {
    let mut name = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let init = summary.clone();
    use_effect(move || {
        if open() {
            name.set(init.name.clone());
            description.set(init.description.clone());
        }
    });
    let id = summary.id;
    let mut open2 = open;
    rsx! {
        Modal { open, title: tr("Playlist bearbeiten"),
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    let n = name().trim().to_string();
                    if n.is_empty() || busy() { return; }
                    let body = PlaylistUpdate { name: Some(n), description: Some(description().trim().to_string()) };
                    spawn(async move {
                        busy.set(true);
                        match api::patch::<PlaylistSummary, _>(&format!("/playlists/{id}"), &body).await {
                            Ok(_) => {
                                toast(tr("Gespeichert."));
                                state::bump_playlists();
                                open2.set(false);
                            }
                            Err(e) => toast_error(e),
                        }
                        busy.set(false);
                    });
                },
                Field { label: tr("Name"),
                    input { class: INPUT, value: "{name}", oninput: move |e| name.set(e.value()) }
                }
                Field { label: tr("Beschreibung"),
                    textarea { class: TEXTAREA, rows: "4", value: "{description}", oninput: move |e| description.set(e.value()) }
                }
                div { class: "flex justify-end gap-2",
                    button { r#type: "button", class: btn(BTN_OUTLINE), onclick: move |_| open2.set(false), {tr("Abbrechen")} }
                    button { r#type: "submit", class: btn(BTN_PRIMARY), disabled: busy() || name().trim().is_empty(), {tr("Speichern")} }
                }
            }
        }
    }
}
