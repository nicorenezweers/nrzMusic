//! Import im Hintergrund (`POST /spotify/import`) für ein Konto. Quellen:
//!
//! - `"liked"` – alle Lieblingssongs (Anmeldung nötig)
//! - `"all"` – Lieblingssongs und alle Playlists (Anmeldung nötig); Playlists, deren Titel
//!   die Web-API nicht herausgibt, kommen über die öffentliche Einbettungsseite
//! - eine Playlist (ID, Link, URI) – mit Anmeldung über die Web-API, sonst (oder wenn die
//!   API sie nicht herausgibt) über die öffentliche Einbettungsseite
//!
//! Jeder Titel wird mit YouTube Music abgeglichen (`SpotifyProgress { task: "match" }`).
//! Mit `create_playlist` entsteht je Quelle eine nrzMusic-Playlist – ein erneuter Import
//! derselben Quelle aktualisiert sie. Bei `"all"` meldet `task: "import_all"` den Fortschritt
//! über die Playlists. Zum Schluss genau ein `SpotifyProgress { task: "import", done:
//! <Treffer>, total: <Titel> }` (oder `task: "import_failed"`) und `PlaylistsChanged`.

use super::SpotifyService;
use crate::db::{playable, playlists};
use crate::error::{AppError, AppResult};
use nrzmusic_spotify::Error as SpError;
use nrzmusic_types::{ServerEvent, SpImportRequest, SpPlaylist, SpTrack, TrackRef, YtTrack};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Höchstens so viele Lieblingssongs werden importiert.
pub const MAX_LIKED: usize = 10_000;
/// Höchstens so oft ein Fortschritts-Ereignis beim Abgleich.
const PROGRESS_EVERY: Duration = Duration::from_millis(300);

/// Was importiert wird.
enum Source {
    Liked,
    All,
    Playlist(String),
}

/// Eine geladene Quelle: Kennung für die Playlist (`"liked"` / Playlist-ID), Name, Titel.
struct Loaded {
    key: String,
    name: String,
    tracks: Vec<SpTrack>,
}

/// Prüfen und im Hintergrund starten. Eine einzelne Playlist wird vorher geladen – gibt es
/// sie nicht, kommt der Fehler gleich zurück. 409, wenn für dieses Konto schon ein Import läuft.
pub async fn start(svc: &SpotifyService, uid: i64, req: SpImportRequest) -> AppResult<()> {
    let raw = req.source.trim().to_string();
    let source = match raw.as_str() {
        "liked" => Source::Liked,
        "all" => Source::All,
        other => Source::Playlist(nrzmusic_spotify::playlist_id(other).ok_or_else(|| {
            AppError::bad("source: \"liked\", \"all\" oder eine Spotify-Playlist (ID oder Link).")
        })?),
    };
    if matches!(source, Source::Liked | Source::All) {
        svc.ensure_connected(uid).await?;
    }
    if !svc.settings().youtube_enabled() {
        return Err(AppError::YoutubeOff);
    }
    if req.download {
        svc.downloads().ensure_usable().await?;
    }
    if !svc.inner.importing.lock().unwrap().insert(uid) {
        return Err(AppError::Conflict(
            "Ein Spotify-Import läuft bereits.".into(),
        ));
    }
    let preloaded = match &source {
        Source::Playlist(id) => match load_playlist(svc, uid, id, None).await {
            Ok(p) => Some(p),
            Err(e) => {
                svc.inner.importing.lock().unwrap().remove(&uid);
                return Err(e);
            }
        },
        _ => None,
    };
    let svc = svc.clone();
    tokio::spawn(async move {
        let result = run(&svc, uid, &req, source, preloaded).await;
        svc.inner.importing.lock().unwrap().remove(&uid);
        let events = svc.events();
        match result {
            Ok((found, total)) => {
                tracing::info!(source = %raw, found, total, "Spotify-Import fertig");
                events.send_to(
                    uid,
                    ServerEvent::SpotifyProgress {
                        task: "import".into(),
                        done: found,
                        total,
                    },
                );
            }
            Err(e) => {
                tracing::warn!(source = %raw, "Spotify-Import fehlgeschlagen: {e}");
                events.send_to(
                    uid,
                    ServerEvent::SpotifyProgress {
                        task: "import_failed".into(),
                        done: 0,
                        total: 0,
                    },
                );
            }
        }
        events.send_to(uid, ServerEvent::PlaylistsChanged);
    });
    Ok(())
}

fn loaded(list: SpPlaylist) -> Loaded {
    let name = list.summary.name.trim();
    let name = if name.is_empty() { "Playlist" } else { name };
    Loaded {
        key: list.summary.id.clone(),
        name: format!("Spotify – {name}"),
        tracks: list.tracks,
    }
}

/// Eine Playlist laden: über die Web-API, wenn verbunden und lesbar (`readable` aus der
/// Übersicht, `None` = ausprobieren), sonst über die öffentliche Einbettungsseite.
async fn load_playlist(
    svc: &SpotifyService,
    uid: i64,
    id: &str,
    readable: Option<bool>,
) -> AppResult<Loaded> {
    let connected = svc.ensure_connected(uid).await.is_ok();
    let mut api_error = None;
    if connected && readable != Some(false) {
        let api = svc.api().clone();
        let pid = id.to_string();
        match svc
            .call(uid, |t| {
                let (api, pid) = (api.clone(), pid.clone());
                async move { api.playlist(&t, &pid).await }
            })
            .await
        {
            Ok(list) => return Ok(loaded(list)),
            // Nicht freigegeben oder nicht gefunden: öffentlich versuchen
            Err(e @ AppError::Spotify(SpError::Api {
                status: 403 | 404, ..
            })) => api_error = Some(e),
            Err(e) => return Err(e),
        }
    }
    let list = match svc.api().public_playlist(id).await {
        Ok(l) => l,
        // Auch öffentlich nicht zu haben: die Meldung der Web-API ist die hilfreichere
        Err(e) => return Err(api_error.unwrap_or(e.into())),
    };
    let mut l = loaded(list);
    l.key = id.to_string();
    Ok(l)
}

/// Alle Lieblingssongs.
async fn load_liked(svc: &SpotifyService, uid: i64) -> AppResult<Loaded> {
    let api = svc.api().clone();
    let tracks = svc
        .call(uid, |t| {
            let api = api.clone();
            async move { api.all_liked(&t, MAX_LIKED).await }
        })
        .await?;
    Ok(Loaded {
        key: "liked".into(),
        name: "Spotify – Lieblingssongs".into(),
        tracks,
    })
}

/// Liefert (Treffer, Titel) über alle Quellen.
async fn run(
    svc: &SpotifyService,
    uid: i64,
    req: &SpImportRequest,
    source: Source,
    preloaded: Option<Loaded>,
) -> AppResult<(u64, u64)> {
    match source {
        Source::Playlist(_) => {
            let l = preloaded.ok_or(AppError::NotFound)?;
            import_one(svc, uid, req, l).await
        }
        Source::Liked => {
            let l = load_liked(svc, uid).await?;
            import_one(svc, uid, req, l).await
        }
        Source::All => {
            let lists = svc.playlists(uid).await?;
            let count = lists.len() as u64 + 1;
            let step = |done: u64| {
                svc.events().send_to(
                    uid,
                    ServerEvent::SpotifyProgress {
                        task: "import_all".into(),
                        done,
                        total: count,
                    },
                )
            };
            step(0);
            let (mut found, mut total) = (0, 0);
            let liked = load_liked(svc, uid).await?;
            let (f, t) = import_one(svc, uid, req, liked).await?;
            (found, total) = (found + f, total + t);
            step(1);
            for (i, p) in lists.iter().enumerate() {
                match load_playlist(svc, uid, &p.id, Some(p.readable)).await {
                    Ok(l) => match import_one(svc, uid, req, l).await {
                        Ok((f, t)) => (found, total) = (found + f, total + t),
                        Err(e) => tracing::warn!(playlist = %p.id, "Import: {e}"),
                    },
                    Err(e) => tracing::warn!(playlist = %p.id, "Playlist laden: {e}"),
                }
                step(i as u64 + 2);
            }
            Ok((found, total))
        }
    }
}

/// Eine Quelle abgleichen, als Playlist übernehmen, auf Wunsch herunterladen.
async fn import_one(
    svc: &SpotifyService,
    uid: i64,
    req: &SpImportRequest,
    l: Loaded,
) -> AppResult<(u64, u64)> {
    let total = l.tracks.len() as u64;
    let events = svc.events().clone();
    let last = Mutex::new(Instant::now() - PROGRESS_EVERY);
    let progress = move |done: u64, all: u64| {
        let mut last = last.lock().unwrap();
        if done == all || done == 0 || last.elapsed() >= PROGRESS_EVERY {
            *last = Instant::now();
            events.send_to(
                uid,
                ServerEvent::SpotifyProgress {
                    task: "match".into(),
                    done,
                    total: all,
                },
            );
        }
    };
    let matched = svc.match_tracks(l.tracks, &progress).await?;
    let found: Vec<YtTrack> = matched.into_iter().filter_map(|t| t.youtube).collect();

    if req.create_playlist && !found.is_empty() {
        let mut keys = Vec::with_capacity(found.len());
        for t in &found {
            if let Some(key) = playable::accept(svc.pool(), &TrackRef::Youtube(t.clone())).await? {
                keys.push(key);
            }
        }
        let description = format!(
            "Aus Spotify übernommen: {} von {} Titeln gefunden.",
            found.len(),
            total
        );
        playlists::upsert_spotify(svc.pool(), uid, &l.key, &l.name, &description, &keys).await?;
    }
    if req.download && !found.is_empty() {
        let mut unique = Vec::with_capacity(found.len());
        let mut seen = std::collections::HashSet::new();
        for t in &found {
            if seen.insert(t.video_id.clone()) {
                unique.push(t.clone());
            }
        }
        svc.downloads().enqueue(&unique).await?;
    }
    Ok((found.len() as u64, total))
}
