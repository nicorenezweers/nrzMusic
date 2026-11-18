//! Globaler Zustand ausser dem Player: Bibliotheksstand (für Live-Aktualisierung), Status,
//! Favoriten-Schlüssel und die Playlists für die Seitenleiste.

use crate::api;
use crate::components::overlay::{toast, toast_error};
use crate::i18n::{tr, trf};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::{LibraryStatus, PlayableTrack, PlaylistSummary, ScanProgress, TrackRef};
use std::collections::HashSet;

/// Wird bei `library_changed`/`scan_finished` erhöht – Seiten laden dann neu.
pub static LIBRARY_VERSION: GlobalSignal<u64> = Signal::global(|| 0);
/// Wird erhöht, wenn sich Playlists geändert haben.
pub static PLAYLISTS_VERSION: GlobalSignal<u64> = Signal::global(|| 0);
/// Wird erhöht, wenn sich Favoriten geändert haben (für die Favoriten-Seite).
pub static FAVORITES_VERSION: GlobalSignal<u64> = Signal::global(|| 0);
/// Wird erhöht, wenn ein Titel gezählt wurde (Verlauf, "Zuletzt gehört").
pub static HISTORY_VERSION: GlobalSignal<u64> = Signal::global(|| 0);

/// Wird erhöht, wenn eine Wiedergabe gemeldet wurde (Statistik).
pub static STATS_VERSION: GlobalSignal<u64> = Signal::global(|| 0);
/// Konto-Einstellung: YouTube-Titel beim Liken herunterladen (der Server lädt sie).
pub static AUTO_DOWNLOAD_LIKES: GlobalSignal<bool> = Signal::global(|| false);

/// Letzter bekannter Status des Servers.
pub static STATUS: GlobalSignal<Option<LibraryStatus>> = Signal::global(|| None);
/// Laufender Scan (None = keiner).
pub static SCAN: GlobalSignal<Option<ScanProgress>> = Signal::global(|| None);
/// Alle Favoriten-Schlüssel (`local:<id>`, `yt:<videoId>`).
pub static FAVORITES: GlobalSignal<HashSet<String>> = Signal::global(HashSet::new);
/// Playlists (Seitenleiste, "Zur Playlist hinzufügen").
pub static PLAYLISTS: GlobalSignal<Vec<PlaylistSummary>> = Signal::global(Vec::new);
/// Verbindung zu den Live-Ereignissen steht.
pub static LIVE: GlobalSignal<bool> = Signal::global(|| false);

pub fn bump_library() {
    *LIBRARY_VERSION.write() += 1;
}

pub fn bump_playlists() {
    *PLAYLISTS_VERSION.write() += 1;
    spawn_forever(load_playlists());
}

/// YouTube Music auf dem Server eingeschaltet (bis zum ersten Status: ja).
pub fn youtube_enabled() -> bool {
    STATUS.read().as_ref().is_none_or(|s| s.youtube_enabled)
}

pub async fn load_status() {
    if let Ok(s) = api::get::<LibraryStatus>("/status").await {
        let scan = if s.scanning { Some(s.scan_progress.unwrap_or_default()) } else { None };
        *SCAN.write() = scan;
        *STATUS.write() = Some(s);
    }
}

pub async fn load_playlists() {
    if let Ok(p) = api::get::<Vec<PlaylistSummary>>("/playlists").await {
        *PLAYLISTS.write() = p;
    }
}

pub async fn load_prefs() {
    if let Ok(p) = api::get::<nrzmusic_types::UserPrefs>("/prefs").await {
        *AUTO_DOWNLOAD_LIKES.write() = p.auto_download_likes;
    }
}

pub async fn load_favorites() {
    if let Ok(keys) = api::get::<Vec<String>>("/favorites/keys").await {
        *FAVORITES.write() = keys.into_iter().collect();
    }
}

pub fn is_favorite(key: &str) -> bool {
    FAVORITES.read().contains(key)
}

/// Herz umschalten (sofort sichtbar, bei Fehler zurück).
pub fn toggle_favorite(track: PlayableTrack) {
    let key = track.key();
    let was = is_favorite(&key);
    if was {
        FAVORITES.write().remove(&key);
    } else {
        FAVORITES.write().insert(key.clone());
    }
    spawn_forever(async move {
        let res = if was {
            api::delete(&format!("/favorites/{}", api::enc(&key))).await
        } else {
            api::put::<serde_json::Value, _>("/favorites", &track.to_ref()).await.map(|_| ())
        };
        match res {
            Ok(()) => {
                *FAVORITES_VERSION.write() += 1;
                if !was {
                    let yt = matches!(track, PlayableTrack::Youtube(_));
                    if yt && *AUTO_DOWNLOAD_LIKES.peek() && crate::downloads::available() {
                        toast(trf("„{0}“ gelikt – wird heruntergeladen.", &[&track.title()]));
                        // Der Server legt den Auftrag an; die Liste kommt über die Live-Ereignisse
                    } else {
                        toast(trf("„{0}“ zu den Favoriten hinzugefügt.", &[&track.title()]));
                    }
                }
            }
            Err(e) => {
                if was {
                    FAVORITES.write().insert(key);
                } else {
                    FAVORITES.write().remove(&key);
                }
                toast_error(e);
            }
        }
    });
}

/// Titel an eine Playlist anhängen.
pub async fn add_to_playlist(id: i64, name: &str, items: Vec<TrackRef>) {
    let n = items.len();
    match api::post::<PlaylistSummary, _>(&format!("/playlists/{id}/items"), &nrzmusic_types::AddItems { items }).await {
        Ok(_) => {
            toast(if n == 1 { trf("Zu „{0}“ hinzugefügt.", &[&name]) } else { trf("{0} Titel zu „{1}“ hinzugefügt.", &[&n, &name]) });
            bump_playlists();
        }
        Err(e) => toast_error(e),
    }
}

/// Neue Playlist anlegen (optional gleich mit Titeln).
pub async fn create_playlist(name: String, description: String, items: Vec<TrackRef>) -> Option<PlaylistSummary> {
    let body = nrzmusic_types::NewPlaylist { name, description, items };
    match api::post::<PlaylistSummary, _>("/playlists", &body).await {
        Ok(p) => {
            toast(trf("Playlist „{0}“ angelegt.", &[&p.name]));
            bump_playlists();
            Some(p)
        }
        Err(e) => {
            toast_error(e);
            None
        }
    }
}

/// Ganzer Musikordner neu einlesen.
pub async fn rescan() {
    match api::action("/library/rescan").await {
        Ok(()) => {
            toast(tr("Der Musikordner wird neu eingelesen."));
            if SCAN.read().is_none() {
                *SCAN.write() = Some(ScanProgress::default());
            }
        }
        Err(e) => toast_error(e),
    }
}
