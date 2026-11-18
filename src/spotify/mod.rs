//! Spotify im Server: Anmeldung mit PKCE (eigene Spotify-App, Client-ID) oder mit dem
//! Webplayer-Cookie `sp_dc` (ohne App und ohne Premium), Tokens nur in der Datenbank und
//! automatisch erneuert, Lieblingssongs und Playlists mit dem zwischengespeicherten
//! Abgleich gegen YouTube Music, Import als nrzMusic-Playlist (und auf Wunsch Downloads).
//!
//! [`SpotifyService`] ist der gemeinsame Zustand (Rocket-State, billig zu klonen).

pub mod import;
pub mod matching;

use crate::db::settings::SettingsStore;
use crate::db::spotify as store;
use crate::downloads::Downloads;
use crate::error::{AppError, AppResult};
use crate::library::Events;
use futures::StreamExt;
use nrzmusic_spotify::{
    parse_cookie, CallbackParams, Error as SpError, PendingLogin, Spotify, Tokens, WEB_PLAYER_SCOPE,
};
use nrzmusic_types::{
    Page, ServerEvent, SpPlaylist, SpPlaylistSummary, SpTrack, SpotifyLogin, SpotifyStatus,
};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use matching::{SongSearch, YtSongSearch};

/// So lange gilt eine begonnene Anmeldung.
const PENDING_TTL: Duration = Duration::from_secs(15 * 60);
/// So viele Sekunden vor Ablauf wird das Access-Token erneuert.
const REFRESH_MARGIN: i64 = 60;
/// Gleichzeitige Abgleiche (je bis zu zwei Suchen nacheinander).
const PARALLEL_MATCHES: usize = 4;
/// Höchstens so viele Titel je `POST /spotify/match`.
pub const MAX_MATCH_TRACKS: usize = 500;

#[derive(Clone)]
pub struct SpotifyService {
    inner: Arc<Inner>,
}

struct Inner {
    api: Arc<Spotify>,
    pool: SqlitePool,
    events: Events,
    settings: SettingsStore,
    search: Arc<dyn SongSearch>,
    downloads: Downloads,
    default_redirect: String,
    /// Begonnene Anmeldungen nach `state` (mit dem Konto, das sie begonnen hat)
    pending: Mutex<HashMap<String, (PendingLogin, i64, Instant)>>,
    /// Zuletzt eingerichtete Client-ID (eine andere macht die Anmeldung ungültig)
    client_id: Mutex<String>,
    refresh_lock: tokio::sync::Mutex<()>,
    /// Konten mit laufendem Import
    importing: Mutex<HashSet<i64>>,
}

fn not_configured() -> AppError {
    AppError::Unavailable(
        "Spotify ist nicht eingerichtet – Client-ID in den Einstellungen eintragen.".into(),
    )
}

fn not_connected() -> AppError {
    AppError::Unauthorized("Nicht mit Spotify verbunden.".into())
}

fn revoked() -> AppError {
    AppError::Unauthorized(
        "Spotify hat die Anmeldung widerrufen oder sie ist abgelaufen – bitte neu verbinden."
            .into(),
    )
}

fn cookie_rejected() -> AppError {
    AppError::bad(
        "Spotify hat das Cookie nicht angenommen – auf open.spotify.com angemeldet? \
         Bitte den aktuellen Wert von „sp_dc“ einfügen.",
    )
}

/// Anmeldung über das Webplayer-Cookie (statt der eigenen Spotify-App).
fn is_cookie(t: &Tokens) -> bool {
    t.scope == WEB_PLAYER_SCOPE
}

/// Spotify-IDs (Titel, Playlists) sind kurz und alphanumerisch.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl SpotifyService {
    pub fn new(
        api: Arc<Spotify>,
        pool: SqlitePool,
        events: Events,
        settings: SettingsStore,
        search: Arc<dyn SongSearch>,
        downloads: Downloads,
        default_redirect: String,
    ) -> Self {
        SpotifyService {
            inner: Arc::new(Inner {
                api,
                pool,
                events,
                settings,
                search,
                downloads,
                default_redirect,
                pending: Mutex::new(HashMap::new()),
                client_id: Mutex::new(String::new()),
                refresh_lock: tokio::sync::Mutex::new(()),
                importing: Mutex::new(HashSet::new()),
            }),
        }
    }

    fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    /// Redirect-URI, wie sie in der Spotify-App eingetragen sein muss.
    pub fn redirect_uri(&self) -> String {
        let saved = self.inner.settings.get().spotify_redirect_uri;
        if saved.trim().is_empty() {
            self.inner.default_redirect.clone()
        } else {
            saved.trim().to_string()
        }
    }

    /// Client-ID und Redirect-URI aus den Einstellungen übernehmen (Start und jede Änderung).
    /// Wechselt die Client-ID, gehören die Tokens zu einer anderen App – abmelden.
    pub async fn configure(&self) -> AppResult<()> {
        let client_id = self
            .inner
            .settings
            .get()
            .spotify_client_id
            .trim()
            .to_string();
        self.inner.api.configure(&client_id, &self.redirect_uri());
        let previous = std::mem::replace(
            &mut *self.inner.client_id.lock().unwrap(),
            client_id.clone(),
        );
        if !previous.is_empty() && !client_id.is_empty() && previous != client_id {
            tracing::info!("Spotify-Client-ID geändert – Anmeldung verworfen");
            self.inner.pending.lock().unwrap().clear();
            if store::delete_all_auth(self.pool()).await? {
                self.inner.events.send(ServerEvent::SpotifyChanged);
            }
        }
        Ok(())
    }

    pub fn is_configured(&self) -> bool {
        self.inner.api.is_configured()
    }

    /// `GET /spotify/status`. Der Benutzer kommt aus dem Zwischenspeicher; fehlt er, wird er
    /// einmal bei Spotify nachgefragt.
    pub async fn status(&self, uid: i64) -> AppResult<SpotifyStatus> {
        let settings = self.inner.settings.get();
        let mut status = SpotifyStatus {
            configured: self.is_configured(),
            connected: false,
            user: None,
            client_id: settings.spotify_client_id.trim().to_string(),
            redirect_uri: self.redirect_uri(),
            via_cookie: false,
        };
        let Some(tokens) = store::tokens(self.pool(), uid).await? else {
            return Ok(status);
        };
        status.via_cookie = is_cookie(&tokens);
        status.user = store::user(self.pool(), uid).await?;
        if status.user.is_none() && (status.configured || status.via_cookie) {
            let api = self.inner.api.clone();
            match self
                .call(uid, |t| {
                    let api = api.clone();
                    async move { api.me(&t).await }
                })
                .await
            {
                Ok(user) => {
                    store::save_user(self.pool(), uid, Some(&user)).await?;
                    status.user = Some(user);
                }
                Err(AppError::Unauthorized(_)) => {}
                Err(e) => tracing::warn!("Spotify-Benutzer: {e}"),
            }
        }
        status.connected = store::tokens(self.pool(), uid).await?.is_some();
        if !status.connected {
            status.user = None;
            status.via_cookie = false;
        }
        Ok(status)
    }

    /// Anmeldung beginnen: Adresse der Spotify-Anmeldeseite.
    pub fn login(&self, uid: i64) -> AppResult<SpotifyLogin> {
        if !self.is_configured() {
            return Err(not_configured());
        }
        let (authorize_url, pending) = self.inner.api.authorize_url()?;
        let mut map = self.inner.pending.lock().unwrap();
        map.retain(|_, (_, _, at)| at.elapsed() < PENDING_TTL);
        map.insert(pending.state.clone(), (pending, uid, Instant::now()));
        Ok(SpotifyLogin { authorize_url })
    }

    /// Anmeldung abschliessen (Rückruf von Spotify oder eingefügte Adresse). Fehler sind
    /// 400 mit einer Meldung für den Benutzer.
    ///
    /// `uid`: das angemeldete Konto (eingefügte Adresse) – es muss das sein, das die Anmeldung
    /// begonnen hat. `None` beim Rückruf von Spotify: Der Browser bringt dort nicht immer das
    /// Sitzungs-Cookie mit (z. B. angemeldet unter einer anderen Adresse als der Redirect-URI);
    /// das Konto ergibt sich dann aus dem zufälligen `state`, den nur der Server und Spotify
    /// kennen.
    pub async fn finish(&self, uid: Option<i64>, params: CallbackParams) -> AppResult<()> {
        if !self.is_configured() {
            return Err(not_configured());
        }
        let (code, state) = match params {
            CallbackParams::Error { error, state } => {
                if let Some(state) = state {
                    self.inner.pending.lock().unwrap().remove(&state);
                }
                return Err(AppError::bad(if error == "access_denied" {
                    "Die Anmeldung bei Spotify wurde abgelehnt.".to_string()
                } else {
                    format!(
                        "Spotify meldet einen Fehler: {}",
                        error.chars().take(100).collect::<String>()
                    )
                }));
            }
            CallbackParams::Code { code, state } => (code, state),
        };
        let pending = self
            .inner
            .pending
            .lock()
            .unwrap()
            .remove(&state)
            // Nur das Konto, das die Anmeldung begonnen hat, darf sie abschliessen
            .filter(|(_, owner, at)| at.elapsed() < PENDING_TTL && uid.is_none_or(|u| u == *owner))
            .map(|(p, owner, _)| (p, owner))
            .ok_or_else(|| {
                AppError::bad(
                    "Unbekannte oder abgelaufene Anmeldung – bitte erneut mit Spotify verbinden.",
                )
            })?;
        let (pending, uid) = pending;
        let tokens = match self.inner.api.exchange_code(&code, &pending.verifier).await {
            Ok(t) => t,
            Err(
                SpError::Unauthorized
                | SpError::Api {
                    status: 400..=499, ..
                },
            ) => {
                return Err(AppError::bad(
                    "Spotify hat den Anmeldecode nicht angenommen – bitte erneut verbinden.",
                ))
            }
            Err(e) => return Err(e.into()),
        };
        let user = match self.inner.api.me(&tokens.access_token).await {
            Ok(u) => Some(u),
            // Entwicklungsmodus: Konto nicht in der App eingetragen – jede Anfrage würde
            // scheitern, also gar nicht erst verbinden (die Meldung erklärt es)
            Err(e @ SpError::Api { status: 403, .. }) => return Err(e.into()),
            Err(e) => {
                tracing::warn!("Spotify-Benutzer nach der Anmeldung: {e}");
                None
            }
        };
        store::save_login(self.pool(), uid, &tokens, user.as_ref()).await?;
        tracing::info!(
            user = %user.as_ref().map(|u| u.id.as_str()).unwrap_or("?"),
            "Mit Spotify verbunden"
        );
        self.inner.events.send_to(uid, ServerEvent::SpotifyChanged);
        Ok(())
    }

    /// Mit dem Webplayer-Cookie `sp_dc` verbinden: gleich ein Token holen (prüft das Cookie)
    /// und das Profil lesen. Ersetzt eine bestehende Anmeldung dieses Kontos.
    pub async fn connect_cookie(&self, uid: i64, input: &str) -> AppResult<()> {
        let sp_dc = parse_cookie(input).ok_or_else(|| {
            AppError::bad(
                "Das sieht nicht nach dem Cookie „sp_dc“ aus – bitte nur dessen Wert einfügen \
                 (eine lange Zeichenkette aus Buchstaben, Ziffern, - und _).",
            )
        })?;
        let tokens = match self.inner.api.web_token(&sp_dc).await {
            Ok(t) => t,
            Err(SpError::Unauthorized) => return Err(cookie_rejected()),
            Err(e) => return Err(e.into()),
        };
        let user = match self.inner.api.me(&tokens.access_token).await {
            Ok(u) => Some(u),
            Err(SpError::Unauthorized) => return Err(cookie_rejected()),
            Err(e) => {
                tracing::warn!("Spotify-Benutzer nach der Anmeldung: {e}");
                None
            }
        };
        store::save_login(self.pool(), uid, &tokens, user.as_ref()).await?;
        tracing::info!(
            user = %user.as_ref().map(|u| u.id.as_str()).unwrap_or("?"),
            "Mit Spotify verbunden (Webplayer-Cookie)"
        );
        self.inner.events.send_to(uid, ServerEvent::SpotifyChanged);
        Ok(())
    }

    /// Abmelden (Tokens löschen).
    pub async fn disconnect(&self, uid: i64) -> AppResult<()> {
        if store::delete_auth(self.pool(), uid).await? {
            tracing::info!("Von Spotify getrennt");
        }
        self.inner.events.send_to(uid, ServerEvent::SpotifyChanged);
        Ok(())
    }

    /// Eingerichtet und angemeldet? Sonst 503 bzw. 401.
    pub async fn ensure_connected(&self, uid: i64) -> AppResult<()> {
        match store::tokens(self.pool(), uid).await? {
            Some(t) if is_cookie(&t) => Ok(()),
            Some(_) if !self.is_configured() => Err(not_configured()),
            Some(_) => Ok(()),
            None if !self.is_configured() => Err(not_configured()),
            None => Err(not_connected()),
        }
    }

    /// Spotify hat die Anmeldung abgelehnt: Tokens weg, Oberfläche benachrichtigen.
    async fn drop_revoked(&self, uid: i64) -> AppError {
        tracing::warn!("Spotify-Anmeldung widerrufen – getrennt");
        if let Err(e) = store::delete_auth(self.pool(), uid).await {
            return e;
        }
        self.inner.events.send_to(uid, ServerEvent::SpotifyChanged);
        revoked()
    }

    /// Erneuern (unter der Sperre). Ist das gespeicherte Token schon ein anderes als `stale`,
    /// hat es inzwischen jemand erneuert.
    async fn refresh(&self, uid: i64, stale: Option<&str>) -> AppResult<String> {
        let _guard = self.inner.refresh_lock.lock().await;
        let tokens: Tokens = store::tokens(self.pool(), uid)
            .await?
            .ok_or_else(not_connected)?;
        let fresh = !tokens.expires_within(REFRESH_MARGIN);
        match stale {
            Some(old) if tokens.access_token != old && fresh => return Ok(tokens.access_token),
            None if fresh => return Ok(tokens.access_token),
            _ => {}
        }
        let renewed = if is_cookie(&tokens) {
            self.inner.api.web_token(&tokens.refresh_token).await
        } else if !self.is_configured() {
            return Err(not_configured());
        } else {
            self.inner.api.refresh(&tokens.refresh_token).await
        };
        match renewed {
            Ok(new) => {
                store::save_tokens(self.pool(), uid, &new).await?;
                Ok(new.access_token)
            }
            Err(SpError::Unauthorized) => Err(self.drop_revoked(uid).await),
            Err(e) => Err(e.into()),
        }
    }

    /// Gültiges Access-Token (kurz vor Ablauf erneuert).
    async fn access_token(&self, uid: i64) -> AppResult<String> {
        self.refresh(uid, None).await
    }

    /// Spotify-Aufruf mit Token; bei `Unauthorized` einmal erneuern und wiederholen.
    pub async fn call<T, F, Fut>(&self, uid: i64, f: F) -> AppResult<T>
    where
        F: Fn(String) -> Fut,
        Fut: Future<Output = nrzmusic_spotify::Result<T>>,
    {
        let token = self.access_token(uid).await?;
        match f(token.clone()).await {
            Err(SpError::Unauthorized) => {
                let token = self.refresh(uid, Some(&token)).await?;
                Ok(f(token).await?)
            }
            other => Ok(other?),
        }
    }

    /// `youtube` aus dem Abgleich-Zwischenspeicher füllen.
    pub async fn fill_matches(&self, tracks: &mut [SpTrack]) -> AppResult<()> {
        let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
        let cached = store::matches(self.pool(), &ids).await?;
        for t in tracks.iter_mut() {
            if let Some(Some(yt)) = cached.get(&t.id) {
                t.youtube = Some(yt.clone());
            }
        }
        Ok(())
    }

    pub async fn liked(&self, uid: i64, offset: u32, limit: u32) -> AppResult<Page<SpTrack>> {
        self.ensure_connected(uid).await?;
        let api = self.inner.api.clone();
        let mut page = self
            .call(uid, |t| {
                let api = api.clone();
                async move { api.liked(&t, offset, limit).await }
            })
            .await?;
        self.fill_matches(&mut page.items).await?;
        Ok(page)
    }

    /// Spotify-ID des angemeldeten Nutzers (zwischengespeichert, sonst einmal nachgefragt).
    async fn user_id(&self, uid: i64) -> AppResult<Option<String>> {
        if let Some(user) = store::user(self.pool(), uid).await? {
            return Ok(Some(user.id));
        }
        let api = self.inner.api.clone();
        let user = self
            .call(uid, |t| {
                let api = api.clone();
                async move { api.me(&t).await }
            })
            .await?;
        store::save_user(self.pool(), uid, Some(&user)).await?;
        Ok(Some(user.id))
    }

    /// Eigene und gefolgte Playlists; `readable` sagt, ob Spotify ihre Titel herausgibt (im
    /// Entwicklungsmodus nur eigene und kollaborative, mit dem Webplayer-Cookie alle).
    pub async fn playlists(&self, uid: i64) -> AppResult<Vec<SpPlaylistSummary>> {
        self.ensure_connected(uid).await?;
        let via_cookie = store::tokens(self.pool(), uid)
            .await?
            .is_some_and(|t| is_cookie(&t));
        let api = self.inner.api.clone();
        let entries = self
            .call(uid, |t| {
                let api = api.clone();
                async move { api.playlist_entries(&t).await }
            })
            .await?;
        // Mit dem Webplayer-Cookie gibt Spotify die Titel aller Playlists heraus
        let user_id = if via_cookie {
            None
        } else {
            match self.user_id(uid).await {
                Ok(id) => id,
                Err(e @ AppError::Unauthorized(_)) => return Err(e),
                Err(e) => {
                    tracing::warn!("Spotify-Benutzer: {e}");
                    None
                }
            }
        };
        Ok(entries
            .into_iter()
            .map(|e| {
                let readable = user_id.as_deref().is_none_or(|id| e.items_readable(id));
                SpPlaylistSummary {
                    readable,
                    ..e.summary
                }
            })
            .collect())
    }

    /// Eine Playlist mit allen Titeln (`youtube` aus dem Abgleich). Gibt Spotify die Titel
    /// nicht heraus, kommt 403 mit der Meldung des Spotify-Clients.
    pub async fn playlist(&self, uid: i64, id: &str) -> AppResult<SpPlaylist> {
        let id = nrzmusic_spotify::playlist_id(id).ok_or(AppError::NotFound)?;
        self.ensure_connected(uid).await?;
        let api = self.inner.api.clone();
        let mut list = self
            .call(uid, |t| {
                let (api, id) = (api.clone(), id.clone());
                async move { api.playlist(&t, &id).await }
            })
            .await?;
        self.fill_matches(&mut list.tracks).await?;
        Ok(list)
    }

    /// Titel mit YouTube Music abgleichen (Zwischenspeicher zuerst, sonst suchen; Treffer und
    /// Fehlschläge werden gespeichert). Gleiche Reihenfolge wie übergeben. `progress(done, total)`
    /// läuft nach jedem Titel.
    pub async fn match_tracks(
        &self,
        mut tracks: Vec<SpTrack>,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> AppResult<Vec<SpTrack>> {
        if !self.inner.settings.youtube_enabled() {
            return Err(AppError::YoutubeOff);
        }
        // Jede Spotify-ID nur einmal suchen
        let mut unique: Vec<SpTrack> = Vec::new();
        let mut seen = HashSet::new();
        for t in &tracks {
            if valid_id(&t.id) && seen.insert(t.id.clone()) {
                unique.push(t.clone());
            }
        }
        let ids: Vec<&str> = unique.iter().map(|t| t.id.as_str()).collect();
        let cached = store::matches(self.pool(), &ids).await?;
        let total = unique.len() as u64;
        let search = self.inner.search.clone();
        let pool = self.pool().clone();
        let mut results = futures::stream::iter(unique.into_iter().map(|t| {
            let hit = cached.get(&t.id).cloned();
            let (search, pool) = (search.clone(), pool.clone());
            async move {
                if let Some(hit) = hit {
                    return (t.id, hit);
                }
                match matching::best_match(search.as_ref(), &t).await {
                    Ok(found) => {
                        let (yt, score) = match found {
                            Some((yt, score)) => (Some(yt), score),
                            None => (None, 0.0),
                        };
                        if let Err(e) = store::save_match(&pool, &t.id, yt.as_ref(), score).await {
                            tracing::warn!("Abgleich speichern: {e}");
                        }
                        (t.id, yt)
                    }
                    Err(e) => {
                        tracing::warn!(spotify = %t.id, "Abgleich mit YouTube Music: {e}");
                        (t.id, None)
                    }
                }
            }
        }))
        .buffer_unordered(PARALLEL_MATCHES);
        let mut found: HashMap<String, Option<nrzmusic_types::YtTrack>> = HashMap::new();
        let mut done = 0u64;
        progress(0, total);
        while let Some((id, yt)) = results.next().await {
            found.insert(id, yt);
            done += 1;
            progress(done, total);
        }
        for t in tracks.iter_mut() {
            t.youtube = found.get(&t.id).cloned().flatten();
        }
        Ok(tracks)
    }

    pub fn events(&self) -> &Events {
        &self.inner.events
    }

    pub fn downloads(&self) -> &Downloads {
        &self.inner.downloads
    }

    pub fn settings(&self) -> &SettingsStore {
        &self.inner.settings
    }

    pub fn api(&self) -> &Arc<Spotify> {
        &self.inner.api
    }
}
