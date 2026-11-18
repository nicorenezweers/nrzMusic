//! Spotify: Verbindungsstatus, Abgleich von Spotify-Titeln mit YouTube Music und Abspielen
//! über die Treffer. Gespielt wird immer der YouTube-Music-Titel (bzw. seine heruntergeladene
//! Datei); Spotify liefert nur die Listen.

use crate::api::{self, ApiError};
use crate::components::overlay::{Ask, confirm, toast, toast_error};
use crate::i18n::{tr, trf};
use crate::{player, util};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::{PlayableTrack, Settings, SpImportRequest, SpMatchRequest, SpTrack, SpotifyStatus, YtTrack};
use std::cell::Cell;
use std::collections::HashMap;

/// Letzter bekannter Status (None = noch nicht geladen).
pub static STATUS: GlobalSignal<Option<SpotifyStatus>> = Signal::global(|| None);
/// Status liess sich nicht laden (Netzwerk o. Ä.).
pub static STATUS_ERROR: GlobalSignal<Option<String>> = Signal::global(|| None);
/// Laufender Abgleich in der Oberfläche: (erledigt, gesamt).
pub static MATCHING: GlobalSignal<Option<(usize, usize)>> = Signal::global(|| None);
/// Hintergrundaufgabe auf dem Server (`spotify_progress`): (Aufgabe, erledigt, gesamt).
/// Der Server meldet `match` während des Abgleichs, am Ende `import` (Treffer, Titel) bzw.
/// `import_failed`.
pub static TASK: GlobalSignal<Option<(String, u64, u64)>> = Signal::global(|| None);
/// Art des hier gestarteten Imports: `Some(true)` = alles herunterladen, `Some(false)` = Playlist.
static IMPORT_KIND: GlobalSignal<Option<bool>> = Signal::global(|| None);
/// Ergebnis des Abgleichs je Spotify-ID: `Some` = gefunden, `None` = kein Treffer.
pub static MATCHES: GlobalSignal<HashMap<String, Option<YtTrack>>> = Signal::global(HashMap::new);

/// Erster Abgleich klein (schneller Start), danach grössere Stücke.
const FIRST_CHUNK: usize = 20;
const CHUNK: usize = 50;
/// Ab so vielen Titeln wird vor Import oder Download nachgefragt.
pub const CONFIRM_FROM: usize = 50;

thread_local! {
    /// Jeder neue Abspielwunsch erhöht das – ein älterer Abgleich hört dann auf.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

pub fn track_url(id: &str) -> String {
    format!("https://open.spotify.com/track/{id}")
}

pub fn playlist_url(id: &str) -> String {
    format!("https://open.spotify.com/playlist/{id}")
}

/// Standard-Redirect-URI des Servers, falls der Status keine liefert (Port aus der Adresse).
pub fn fallback_redirect_uri() -> String {
    let port = web_sys::window()
        .and_then(|w| w.location().port().ok())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "8350".into());
    format!("http://127.0.0.1:{port}/api/v1/spotify/callback")
}

// ------------------------------------------------------------------ Status

pub async fn load_status() {
    match api::get::<SpotifyStatus>("/spotify/status").await {
        Ok(s) => {
            *STATUS.write() = Some(s);
            *STATUS_ERROR.write() = None;
        }
        // Ohne Client-ID antwortet der Server evtl. schon hier mit 503
        Err(e) if e.unavailable() => {
            let old = STATUS.peek().clone().unwrap_or_default();
            *STATUS.write() = Some(SpotifyStatus { configured: false, connected: false, user: None, ..old });
            *STATUS_ERROR.write() = None;
        }
        Err(e) => {
            if STATUS.peek().is_none() {
                *STATUS_ERROR.write() = Some(e.message);
            }
        }
    }
}

pub fn reload_status() {
    spawn_forever(load_status());
}

/// Fehler einer Spotify-Route: 401 (abgemeldet) bzw. 503 (nicht eingerichtet) laden den
/// Status neu – die Seite zeigt dann wieder Verbinden bzw. die Anleitung.
pub fn handle_error(e: &ApiError) {
    if e.unauthorized() || e.unavailable() {
        reload_status();
    }
}

/// Fehler melden und behandeln.
pub fn report(e: ApiError) {
    handle_error(&e);
    toast_error(e);
}

// ------------------------------------------------------------------ Abgleich

/// Bekannter Treffer: `Some(Some(_))` gefunden, `Some(None)` nichts gefunden, `None` noch offen.
pub fn known_match(t: &SpTrack) -> Option<Option<YtTrack>> {
    if let Some(y) = &t.youtube {
        return Some(Some(y.clone()));
    }
    MATCHES.peek().get(&t.id).cloned()
}

/// Wie [`known_match`], aber reaktiv (für die Anzeige in Zeilen).
pub fn match_state(t: &SpTrack) -> Option<Option<YtTrack>> {
    if let Some(y) = &t.youtube {
        return Some(Some(y.clone()));
    }
    MATCHES.read().get(&t.id).cloned()
}

/// Titel ohne bekannten Treffer beim Server abgleichen; Ergebnis landet in [`MATCHES`].
async fn match_part(tracks: &[SpTrack]) -> Result<(), ApiError> {
    let todo: Vec<SpTrack> = tracks.iter().filter(|t| known_match(t).is_none()).cloned().collect();
    if todo.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = todo.iter().map(|t| t.id.clone()).collect();
    let res = api::post::<Vec<SpTrack>, _>("/spotify/match", &SpMatchRequest { tracks: todo }).await?;
    let mut m = MATCHES.write();
    for t in res {
        m.insert(t.id.clone(), t.youtube);
    }
    // Was der Server nicht zurückgibt, gilt als nicht gefunden
    for id in ids {
        m.entry(id).or_insert(None);
    }
    Ok(())
}

fn next_generation() -> u64 {
    GENERATION.with(|g| {
        let n = g.get() + 1;
        g.set(n);
        n
    })
}

fn current_generation() -> u64 {
    GENERATION.with(|g| g.get())
}

/// Alle Titel abgleichen, ohne abzuspielen (Fortschritt in [`MATCHING`]).
pub fn match_all(list: Vec<SpTrack>) {
    spawn_forever(async move {
        let generation = next_generation();
        let total = list.len();
        let mut pos = 0;
        *MATCHING.write() = Some((0, total));
        while pos < total && current_generation() == generation {
            let end = (pos + CHUNK).min(total);
            if let Err(e) = match_part(&list[pos..end]).await {
                report(e);
                break;
            }
            pos = end;
            *MATCHING.write() = Some((pos, total));
        }
        if current_generation() == generation {
            *MATCHING.write() = None;
            let missed = list.iter().filter(|t| matches!(known_match(t), Some(None))).count();
            if missed > 0 {
                toast(trf("Abgleich fertig – {0} Titel ohne Treffer auf YouTube Music.", &[&missed]));
            } else {
                toast(tr("Abgleich fertig – alle Titel gefunden."));
            }
        }
    });
}

/// Liste ab `start` abspielen (bzw. gemischt): stückweise abgleichen, mit dem ersten Treffer
/// starten, den Rest im Hintergrund anhängen. Titel ohne Treffer werden übersprungen.
pub fn play(list: Vec<SpTrack>, start: usize, shuffle: bool) {
    if list.is_empty() {
        return;
    }
    spawn_forever(async move {
        let generation = next_generation();
        let list: Vec<SpTrack> = if shuffle {
            let mut l = list;
            util::shuffle(&mut l);
            l
        } else {
            let s = start.min(list.len() - 1);
            list[s..].to_vec()
        };
        let total = list.len();
        let mut pos = 0;
        let mut chunk = FIRST_CHUNK;
        let mut missed = 0usize;
        let mut started: Option<u64> = None;
        let mut aborted = false;
        *MATCHING.write() = Some((0, total));
        while pos < total {
            if current_generation() != generation {
                aborted = true;
                break;
            }
            let end = (pos + chunk).min(total);
            if let Err(e) = match_part(&list[pos..end]).await {
                report(e);
                aborted = true;
                break;
            }
            let mut found: Vec<PlayableTrack> = list[pos..end].iter().filter_map(|t| known_match(t).flatten()).map(PlayableTrack::Youtube).collect();
            missed += (end - pos) - found.len();
            pos = end;
            chunk = CHUNK;
            if current_generation() == generation {
                *MATCHING.write() = Some((pos, total));
            }
            if found.is_empty() {
                continue;
            }
            match started {
                None => {
                    if shuffle {
                        *player::SHUFFLE.write() = true;
                    }
                    player::play_now(found, 0);
                    started = player::current_uid();
                }
                Some(uid) => {
                    // Inzwischen etwas anderes gestartet: nicht mehr anhängen
                    if !player::queue_contains(uid) {
                        aborted = true;
                        break;
                    }
                    if *player::SHUFFLE.peek() {
                        util::shuffle(&mut found);
                    }
                    player::extend_queue(found);
                }
            }
        }
        if current_generation() == generation {
            *MATCHING.write() = None;
        }
        if aborted {
            return;
        }
        if started.is_none() {
            toast_error(tr("Keiner der Titel wurde auf YouTube Music gefunden."));
        } else if missed > 0 {
            toast(if missed == 1 {
                tr("1 Titel ohne Treffer auf YouTube Music übersprungen.")
            } else {
                trf("{0} Titel ohne Treffer auf YouTube Music übersprungen.", &[&missed])
            });
        }
    });
}

/// Treffer für einen einzelnen Titel (gleicht bei Bedarf ab); meldet, wenn es keinen gibt.
pub async fn resolve(t: &SpTrack) -> Option<YtTrack> {
    if known_match(t).is_none()
        && let Err(e) = match_part(std::slice::from_ref(t)).await
    {
        report(e);
        return None;
    }
    let found = known_match(t).flatten();
    if found.is_none() {
        toast(trf("Kein passender Titel auf YouTube Music für „{0}“.", &[&t.name]));
    }
    found
}

// ------------------------------------------------------------------ Import, Download

/// Als nrzMusic-Playlist übernehmen und/oder alles herunterladen (läuft auf dem Server).
/// `source`: `"liked"` oder eine Playlist-ID; `count` nur für die Rückfrage.
pub async fn import(source: String, name: String, count: usize, create_playlist: bool, download: bool) {
    if count >= CONFIRM_FROM {
        let ask = if download {
            Ask::new(
                trf("Alle {0} Titel herunterladen?", &[&count]),
                tr("Jeder Titel wird auf YouTube Music gesucht und mit yt-dlp in den Musikordner geladen. Das läuft im Hintergrund und kann eine Weile dauern."),
            )
            .confirm(tr("Herunterladen"))
        } else {
            Ask::new(
                trf("„{0}“ als Playlist übernehmen?", &[&name]),
                trf("Für {0} Titel wird der passende Titel auf YouTube Music gesucht. Das läuft im Hintergrund und kann eine Weile dauern.", &[&count]),
            )
            .confirm(tr("Übernehmen"))
        };
        if !confirm(ask).await {
            return;
        }
    }
    let body = SpImportRequest { source, create_playlist, download };
    match api::post::<serde_json::Value, _>("/spotify/import", &body).await {
        Ok(_) => {
            *IMPORT_KIND.write() = Some(download);
            *TASK.write() = Some(("match".into(), 0, count as u64));
            toast(if download {
                tr("Abgleich gestartet – die Treffer werden danach heruntergeladen.")
            } else {
                tr("Import gestartet – die Playlist erscheint, sobald der Abgleich fertig ist.")
            });
        }
        Err(e) => report(e),
    }
}

/// Bezeichnung einer Hintergrundaufgabe des Servers.
pub fn task_label(task: &str) -> String {
    let kind = *IMPORT_KIND.read();
    match (task, kind) {
        ("match", Some(true)) => tr("Alle herunterladen: Abgleich mit YouTube Music"),
        ("match", Some(false)) => tr("Als Playlist übernehmen: Abgleich mit YouTube Music"),
        ("match", None) => tr("Spotify-Import: Abgleich mit YouTube Music"),
        ("import_all", _) => tr("Alles übernehmen: Playlists"),
        _ => tr("Spotify-Import"),
    }
}

/// `spotify_progress`: Fortschritt zeigen; am Ende melden und ausblenden.
pub fn on_progress(task: String, done: u64, total: u64) {
    match task.as_str() {
        "import" => {
            let kind = IMPORT_KIND.write().take();
            *TASK.write() = None;
            let found = trf("{0} von {1} Titeln gefunden", &[&done, &total]);
            match kind {
                _ if done == 0 => toast_error(tr("Spotify-Import fertig: kein Titel auf YouTube Music gefunden – nichts übernommen.")),
                Some(true) => toast(trf("Spotify-Import fertig: {0} – die Treffer werden heruntergeladen.", &[&found])),
                Some(false) => toast(trf("Spotify-Import fertig: {0} – die Playlist ist angelegt.", &[&found])),
                None => toast(trf("Spotify-Import fertig: {0}.", &[&found])),
            }
        }
        "import_failed" => {
            IMPORT_KIND.write().take();
            *TASK.write() = None;
            toast_error(tr("Der Spotify-Import ist fehlgeschlagen – Details stehen im Server-Protokoll."));
        }
        _ => *TASK.write() = Some((task, done, total)),
    }
}

// ------------------------------------------------------------------ Verbindung

/// Client-ID speichern (übrige Einstellungen bleiben) und den Status neu laden.
pub async fn save_client_id(id: String) -> bool {
    let mut s = match api::get::<Settings>("/settings").await {
        Ok(s) => s,
        Err(e) => {
            toast_error(e);
            return false;
        }
    };
    s.spotify_client_id = id;
    match api::put::<Settings, _>("/settings", &s).await {
        Ok(_) => {
            toast(tr("Client-ID gespeichert."));
            load_status().await;
            true
        }
        Err(e) => {
            toast_error(e);
            false
        }
    }
}

/// Verbindung trennen (mit Rückfrage).
pub async fn disconnect() {
    let ask = Ask::new(
        tr("Von Spotify trennen?"),
        tr("Die Anmeldung wird in nrzMusic gelöscht. Übernommene Playlists und heruntergeladene Titel bleiben."),
    )
    .confirm(tr("Trennen"))
    .destructive();
    if !confirm(ask).await {
        return;
    }
    match api::delete("/spotify/session").await {
        Ok(()) => {
            toast(tr("Von Spotify getrennt."));
            load_status().await;
        }
        Err(e) => report(e),
    }
}
