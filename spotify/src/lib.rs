//! Client für die Spotify-Web-API: Anmeldung mit Authorization Code + PKCE (ohne
//! Client-Secret), Lieblingssongs, eigene Playlists – und der Abgleich eines Spotify-Titels
//! mit Treffern von YouTube Music (`match_query`, `match_score`). Liefert die gemeinsamen
//! Typen aus `nrzmusic-types`. Abgespielt wird nichts über Spotify.
//!
//! Regeln der Web-API (Stand Oktober 2026), nach denen sich dieser Client richtet:
//! - **Redirect-URI**: HTTPS, ausser für Loopback-Adressen als IP (`http://127.0.0.1:PORT`,
//!   `http://[::1]:PORT`); `localhost` ist nicht erlaubt. Die URI muss exakt so in der
//!   Spotify-App eingetragen sein.
//! - **Entwicklungsmodus** (jede selbst angelegte App): Besitzer braucht Spotify Premium,
//!   höchstens 5 Nutzer, die vorher in der App unter "User Management" eingetragen werden
//!   (sonst `403` auf jede Anfrage).
//! - **Playlists**: Titel gibt es nur noch für eigene und kollaborative Playlists
//!   (`GET /playlists/{id}/items`, Einträge mit `item` statt `track`); für andere Playlists
//!   nur Metadaten bzw. `403`. Spotify-eigene redaktionelle und algorithmische Playlists
//!   sind für neue Apps ganz gesperrt.
//! - `GET /me` liefert `product` (Premium/Free) im Entwicklungsmodus nicht mehr.
//! - Refresh-Tokens gelten 6 Monate ab der Anmeldung; danach `invalid_grant` → neu anmelden.

mod auth;
mod matching;
mod parse;
mod public;
mod web;

use std::sync::RwLock;
use std::time::Duration;

use reqwest::header::{ACCEPT, RETRY_AFTER};
use reqwest::StatusCode;
use serde_json::Value;

pub use auth::parse_callback;
pub use public::parse_embed;
pub use web::{parse_cookie, DEFAULT_SECRETS_URL, WEB_PLAYER_SCOPE};
pub use matching::{best_match, match_query, match_score, MATCH_THRESHOLD};
pub use nrzmusic_types::{Page, SpPlaylist, SpPlaylistSummary, SpTrack, SpotifyUser, YtTrack};

/// Berechtigungen, die nrzMusic anfragt (nur lesen).
pub const SCOPES: &str =
    "user-library-read playlist-read-private playlist-read-collaborative user-read-private";

/// Höchstens so viele Einträge liefert Spotify pro Seite.
pub const MAX_PAGE_LIMIT: u32 = 50;

const USER_AGENT: &str = concat!("nrzMusic/", env!("CARGO_PKG_VERSION"));
/// Wiederholungen nach `429 Too Many Requests` (und `502`/`503`/`504`).
const MAX_RETRIES: u32 = 3;
/// Längere `Retry-After`-Wartezeiten (z. B. Tageskontingent im Entwicklungsmodus) werden
/// nicht abgewartet, sondern als Fehler gemeldet.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);
/// Schutz gegen endlose Seitenfolgen (500 × 50 = 25 000 Einträge).
const MAX_PAGES: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Spotify nicht erreichbar: {0}")]
    Http(#[from] reqwest::Error),
    /// Token abgelaufen oder widerrufen – neu anmelden bzw. erneuern
    #[error("Spotify-Anmeldung abgelaufen")]
    Unauthorized,
    #[error("Spotify: {message} ({status})")]
    Api { status: u16, message: String },
    #[error("Unerwartete Antwort von Spotify: {0}")]
    Parse(String),
    #[error("Spotify ist nicht eingerichtet (Client-ID fehlt)")]
    NotConfigured,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Tokens nach Anmeldung oder Erneuerung.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix-Sekunden
    pub expires_at: i64,
    pub scope: String,
}

impl Tokens {
    /// Das Access-Token läuft in weniger als `margin_secs` Sekunden ab (oder ist abgelaufen)
    /// – vorher mit [`Spotify::refresh`] erneuern.
    pub fn expires_within(&self, margin_secs: i64) -> bool {
        self.expires_at - margin_secs <= auth::now()
    }
}

/// Begonnene Anmeldung: `state` gegen CSRF, `verifier` für PKCE. Der Server merkt sie sich,
/// bis der Rückruf kommt.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingLogin {
    pub state: String,
    pub verifier: String,
}

/// Ergebnis aus der Rückruf-Adresse.
#[derive(Clone, Debug, PartialEq)]
pub enum CallbackParams {
    Code {
        code: String,
        state: String,
    },
    /// Spotify meldet einen Fehler, z. B. "access_denied"
    Error {
        error: String,
        state: Option<String>,
    },
}

/// Eine Playlist aus `GET /me/playlists` mit den Angaben, die entscheiden, ob Spotify ihre
/// Titel herausgibt.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaylistEntry {
    pub summary: SpPlaylistSummary,
    /// Spotify-Nutzer-ID des Besitzers ("spotify" bei redaktionellen Playlists)
    pub owner_id: Option<String>,
    pub collaborative: bool,
}

impl PlaylistEntry {
    /// Ob [`Spotify::playlist`] die Titel für diesen Nutzer (`SpotifyUser::id`) laden kann.
    /// Apps im Entwicklungsmodus sehen seit 2026 nur Titel eigener und kollaborativer
    /// Playlists; gefolgte Playlists anderer (auch von Spotify) liefern nur Metadaten.
    pub fn items_readable(&self, user_id: &str) -> bool {
        self.collaborative || self.owner_id.as_deref() == Some(user_id)
    }
}

pub struct Spotify {
    http: reqwest::Client,
    client_id: RwLock<String>,
    redirect_uri: RwLock<String>,
    /// `https://accounts.spotify.com` – in Tests ein lokaler Ersatz-Server
    accounts_url: String,
    /// `https://api.spotify.com/v1`
    api_url: String,
    /// `https://open.spotify.com` (Einbettungsseiten öffentlicher Playlists, Webplayer-Tokens)
    web_url: String,
    /// Liste der TOTP-Geheimnisse für Webplayer-Tokens (siehe `web`)
    secrets_url: String,
    secrets: RwLock<web::SecretCache>,
}

impl Spotify {
    pub fn new() -> Self {
        Self::with_endpoints("https://accounts.spotify.com", "https://api.spotify.com/v1")
    }

    /// Andere Adressen für Anmeldung und API (Tests mit einem Ersatz-Server).
    pub fn with_endpoints(accounts_url: &str, api_url: &str) -> Self {
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10));
        if is_loopback(accounts_url) || is_loopback(api_url) {
            // Ein lokaler Ersatz-Server soll nie über einen Proxy angesprochen werden
            builder = builder.no_proxy();
        }
        Spotify {
            http: builder.build().unwrap_or_else(|_| reqwest::Client::new()),
            client_id: RwLock::new(String::new()),
            redirect_uri: RwLock::new(String::new()),
            accounts_url: accounts_url.trim_end_matches('/').to_string(),
            api_url: api_url.trim_end_matches('/').to_string(),
            // Mit einem Ersatz-Server (Tests) auch die Einbettungsseiten von dort
            web_url: if is_loopback(accounts_url) {
                accounts_url.trim_end_matches('/').to_string()
            } else {
                "https://open.spotify.com".into()
            },
            secrets_url: if is_loopback(accounts_url) {
                format!("{}/secretDict.json", accounts_url.trim_end_matches('/'))
            } else {
                std::env::var("NRZMUSIC_SPOTIFY_SECRETS_URL")
                    .ok()
                    .map(|u| u.trim().to_string())
                    .filter(|u| !u.is_empty())
                    .unwrap_or_else(|| DEFAULT_SECRETS_URL.into())
            },
            secrets: RwLock::new(web::SecretCache::default()),
        }
    }

    /// Client-ID und Redirect-URI setzen (beim Start und nach jeder Änderung der Einstellungen).
    pub fn configure(&self, client_id: &str, redirect_uri: &str) {
        *self.client_id.write().unwrap_or_else(|e| e.into_inner()) = client_id.trim().to_string();
        *self.redirect_uri.write().unwrap_or_else(|e| e.into_inner()) =
            redirect_uri.trim().to_string();
    }

    /// Andere Adresse für Einbettungsseiten (Tests).
    pub fn with_web_url(mut self, web_url: &str) -> Self {
        self.web_url = web_url.trim_end_matches('/').to_string();
        self
    }

    /// Öffentliche Playlist ohne Anmeldung (Einbettungsseite, höchstens 100 Titel).
    pub async fn public_playlist(&self, id: &str) -> Result<SpPlaylist> {
        let id = playlist_id(id).ok_or_else(|| Error::Api {
            status: 400,
            message: "Ungültige Spotify-Playlist-ID".into(),
        })?;
        let url = format!("{}/embed/playlist/{id}", self.web_url);
        let resp = self
            .http
            .get(&url)
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0 Safari/537.36",
            )
            .header(reqwest::header::ACCEPT_LANGUAGE, "de,en;q=0.8")
            .send()
            .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Err(Error::Api {
                status: 404,
                message: "Playlist nicht gefunden oder nicht öffentlich".into(),
            });
        }
        if !resp.status().is_success() {
            return Err(Error::Api {
                status: resp.status().as_u16(),
                message: "Spotify hat die Playlist-Seite nicht geliefert".into(),
            });
        }
        let html = resp.text().await?;
        parse_embed(&html, &id)
    }

    pub fn is_configured(&self) -> bool {
        !self.client_id().is_empty()
    }

    fn client_id(&self) -> String {
        self.client_id
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn redirect_uri(&self) -> String {
        self.redirect_uri
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    // ------------------------------------------------------------ Anmeldung

    /// Adresse der Spotify-Anmeldeseite und die dazugehörige offene Anmeldung (neuer
    /// zufälliger `state` und PKCE-Verifier mit 64 Zeichen, Challenge mit S256).
    /// [`Error::NotConfigured`] ohne Client-ID oder Redirect-URI.
    pub fn authorize_url(&self) -> Result<(String, PendingLogin)> {
        let client_id = self.client_id();
        let redirect_uri = self.redirect_uri();
        if client_id.is_empty() || redirect_uri.is_empty() {
            return Err(Error::NotConfigured);
        }
        let verifier = auth::new_verifier();
        let state = auth::new_state();
        let mut url = url::Url::parse(&format!("{}/authorize", self.accounts_url))
            .map_err(|e| Error::Parse(format!("Anmeldeadresse: {e}")))?;
        url.query_pairs_mut()
            .append_pair("client_id", &client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &auth::challenge(&verifier))
            .append_pair("scope", SCOPES)
            .append_pair("state", &state);
        // Leerzeichen als %20 statt "+" (ein echtes "+" ist bereits %2B)
        let url = url.to_string().replace('+', "%20");
        Ok((url, PendingLogin { state, verifier }))
    }

    /// Den Code aus dem Rückruf gegen Tokens tauschen (`code_verifier` aus [`PendingLogin`]).
    /// Fehler von Spotify (falscher Code, falsche Redirect-URI, unbekannte Client-ID) kommen
    /// als [`Error::Api`] mit Spotifys Beschreibung.
    pub async fn exchange_code(&self, code: &str, verifier: &str) -> Result<Tokens> {
        let client_id = self.client_id();
        if client_id.is_empty() {
            return Err(Error::NotConfigured);
        }
        let redirect_uri = self.redirect_uri();
        let (status, body) = self
            .token_request(&[
                ("grant_type", "authorization_code"),
                ("code", code.trim()),
                ("redirect_uri", &redirect_uri),
                ("client_id", &client_id),
                ("code_verifier", verifier),
            ])
            .await?;
        if status.is_success() {
            return auth::tokens_from(&json_body(&body, "Token")?, None, auth::now());
        }
        Err(api_error(status, &body))
    }

    /// Access-Token erneuern. Gibt Spotify kein neues Refresh-Token zurück, bleibt das alte.
    /// [`Error::Unauthorized`], wenn das Refresh-Token abgelaufen oder widerrufen ist
    /// (`invalid_grant`, `400`/`401`) – dann neu anmelden.
    pub async fn refresh(&self, refresh_token: &str) -> Result<Tokens> {
        let client_id = self.client_id();
        if client_id.is_empty() {
            return Err(Error::NotConfigured);
        }
        let refresh_token = refresh_token.trim();
        if refresh_token.is_empty() {
            return Err(Error::Unauthorized);
        }
        let (status, body) = self
            .token_request(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", &client_id),
            ])
            .await?;
        if status.is_success() {
            return auth::tokens_from(
                &json_body(&body, "Token")?,
                Some(refresh_token),
                auth::now(),
            );
        }
        if matches!(status.as_u16(), 400 | 401)
            || parse::auth_error_code(&body).as_deref() == Some("invalid_grant")
        {
            tracing::info!(
                "Spotify: Erneuern abgelehnt ({status}): {}",
                parse::error_message(&body).unwrap_or_default()
            );
            return Err(Error::Unauthorized);
        }
        Err(api_error(status, &body))
    }

    // ------------------------------------------------------------ Webplayer (Cookie)

    /// Webplayer-Token mit dem Cookie `sp_dc` holen (keine Client-ID, kein Premium nötig).
    /// `refresh_token` der Antwort ist das Cookie selbst, `scope` ist [`WEB_PLAYER_SCOPE`].
    /// [`Error::Unauthorized`], wenn Spotify das Cookie nicht (mehr) annimmt.
    pub async fn web_token(&self, sp_dc: &str) -> Result<Tokens> {
        let sp_dc = parse_cookie(sp_dc).ok_or(Error::Unauthorized)?;
        let secret = self.secret(false).await?;
        match self.web_token_with(&sp_dc, &secret).await {
            Err(Error::Api { status, message }) if web::totp_rejected(status, &message) => {
                // Geheimnis vermutlich gewechselt: Liste neu laden, einmal wiederholen
                let newer = self.secret(true).await?;
                if newer.version == secret.version {
                    return Err(totp_error(secret.version));
                }
                match self.web_token_with(&sp_dc, &newer).await {
                    Err(Error::Api { status, message }) if web::totp_rejected(status, &message) => {
                        Err(totp_error(newer.version))
                    }
                    other => other,
                }
            }
            other => other,
        }
    }

    async fn web_token_with(&self, sp_dc: &str, secret: &web::Secret) -> Result<Tokens> {
        let code = web::totp(&secret.key, auth::now());
        let version = secret.version.to_string();
        let url = format!("{}/api/token", self.web_url);
        let resp = self
            .send(|| {
                self.http
                    .get(&url)
                    .query(&[
                        ("reason", "init"),
                        ("productType", "web-player"),
                        ("totp", code.as_str()),
                        ("totpServer", code.as_str()),
                        ("totpVer", version.as_str()),
                    ])
                    .header(reqwest::header::USER_AGENT, web::BROWSER_UA)
                    .header(ACCEPT, "application/json")
                    .header(reqwest::header::COOKIE, format!("sp_dc={sp_dc}"))
            })
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if status.is_success() {
            return web::tokens_from(&json_body(&body, "Webplayer-Token")?, sp_dc, auth::now());
        }
        if matches!(status.as_u16(), 401 | 403) {
            tracing::info!("Spotify: Webplayer-Token abgelehnt ({status})");
            return Err(Error::Unauthorized);
        }
        Err(Error::Api {
            status: status.as_u16(),
            message: parse::error_message(&body).unwrap_or(body),
        })
    }

    /// Aktuelles TOTP-Geheimnis (zwischengespeichert; `force` lädt die Liste neu).
    async fn secret(&self, force: bool) -> Result<web::Secret> {
        if !force {
            if let Some(s) = self.secrets.read().unwrap_or_else(|e| e.into_inner()).fresh() {
                return Ok(s);
            }
        }
        let resp = self
            .send(|| self.http.get(&self.secrets_url).header(ACCEPT, "application/json"))
            .await;
        let loaded = match resp {
            Ok(r) if r.status().is_success() => {
                let body = r.text().await?;
                web::newest_secret(&json_body(&body, "TOTP-Geheimnisse")?)
            }
            Ok(r) => {
                tracing::warn!("TOTP-Geheimnisse: {} von {}", r.status(), self.secrets_url);
                None
            }
            Err(e) => {
                tracing::warn!("TOTP-Geheimnisse: {e}");
                None
            }
        };
        let mut cache = self.secrets.write().unwrap_or_else(|e| e.into_inner());
        match loaded {
            Some(s) => {
                cache.loaded = Some((std::time::Instant::now(), s.clone()));
                Ok(s)
            }
            // Nicht erreichbar: das zuletzt bekannte weiter benutzen
            None => cache.loaded.as_ref().map(|(_, s)| s.clone()).ok_or_else(|| Error::Api {
                status: 503,
                message: format!(
                    "Die aktuellen Webplayer-Schlüssel liessen sich nicht laden ({})",
                    self.secrets_url
                ),
            }),
        }
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<(StatusCode, String)> {
        let url = format!("{}/api/token", self.accounts_url);
        let resp = self
            .send(|| {
                self.http
                    .post(&url)
                    .header(ACCEPT, "application/json")
                    .form(form)
            })
            .await?;
        let status = resp.status();
        Ok((status, resp.text().await?))
    }

    // ------------------------------------------------------------ Web-API

    /// Profil des angemeldeten Nutzers. Ein `403` heisst im Entwicklungsmodus fast immer:
    /// Konto nicht in der App eingetragen oder der App-Besitzer hat kein Premium.
    pub async fn me(&self, access_token: &str) -> Result<SpotifyUser> {
        let url = format!("{}/me", self.api_url);
        match self.get_json(access_token, &url).await {
            Ok(v) => parse::user(&v).ok_or_else(|| Error::Parse("Profil ohne ID".into())),
            Err(Error::Api {
                status: 403,
                message,
            }) => Err(Error::Api {
                status: 403,
                message: format!(
                    "{message} – ist dieses Spotify-Konto in der Spotify-App unter \
                     „User Management“ eingetragen, und hat der Besitzer der App Spotify Premium?"
                ),
            }),
            Err(e) => Err(e),
        }
    }

    /// Lieblingssongs, neueste zuerst. `limit` höchstens 50 (Spotify-Grenze).
    pub async fn liked(
        &self,
        access_token: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Page<SpTrack>> {
        let limit = limit.clamp(1, MAX_PAGE_LIMIT);
        let url = format!("{}/me/tracks?offset={offset}&limit={limit}", self.api_url);
        let page = self.get_json(access_token, &url).await?;
        expect_items(&page, "Lieblingssongs")?;
        let info = parse::page_info(&page);
        let offset = info.offset.unwrap_or(offset as u64);
        Ok(Page {
            items: parse::track_page_items(&page),
            total: info.total.unwrap_or(offset + info.count as u64),
            offset,
            limit: info.limit.unwrap_or(limit as u64),
        })
    }

    /// Alle Lieblingssongs (folgt den Seiten), höchstens `max`.
    pub async fn all_liked(&self, access_token: &str, max: usize) -> Result<Vec<SpTrack>> {
        let mut out = Vec::new();
        if max == 0 {
            return Ok(out);
        }
        let limit = max.min(MAX_PAGE_LIMIT as usize);
        let url = format!("{}/me/tracks?offset=0&limit={limit}", self.api_url);
        let first = self.get_json(access_token, &url).await?;
        self.follow_pages(
            access_token,
            first,
            "/me/tracks",
            "Lieblingssongs",
            |page| {
                out.extend(parse::track_page_items(page));
                out.len() < max
            },
        )
        .await?;
        out.truncate(max);
        Ok(out)
    }

    /// Eigene und gefolgte Playlists (alle Seiten).
    pub async fn playlists(&self, access_token: &str) -> Result<Vec<SpPlaylistSummary>> {
        Ok(self
            .playlist_entries(access_token)
            .await?
            .into_iter()
            .map(|e| e.summary)
            .collect())
    }

    /// Wie [`Spotify::playlists`], dazu Besitzer-ID und "kollaborativ" – daraus folgt, ob
    /// die Titel lesbar sind ([`PlaylistEntry::items_readable`]).
    pub async fn playlist_entries(&self, access_token: &str) -> Result<Vec<PlaylistEntry>> {
        let url = format!(
            "{}/me/playlists?offset=0&limit={MAX_PAGE_LIMIT}",
            self.api_url
        );
        let first = self.get_json(access_token, &url).await?;
        let mut out: Vec<PlaylistEntry> = Vec::new();
        self.follow_pages(access_token, first, "/me/playlists", "Playlists", |page| {
            for entry in page
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(parse::playlist_entry)
            {
                if !out.iter().any(|e| e.summary.id == entry.summary.id) {
                    out.push(entry);
                }
            }
            true
        })
        .await?;
        Ok(out)
    }

    /// Eine Playlist mit allen Titeln (folgt den Seiten; Episoden und lokale Dateien fallen weg).
    ///
    /// `id` darf auch ein Link (`https://open.spotify.com/playlist/…`) oder eine URI
    /// (`spotify:playlist:…`) sein. Gibt Spotify die Titel nicht heraus (fremde Playlist im
    /// Entwicklungsmodus), kommt [`Error::Api`] mit Status `403`; Spotify-eigene Playlists
    /// gibt es oft gar nicht (`404`).
    pub async fn playlist(&self, access_token: &str, id: &str) -> Result<SpPlaylist> {
        let id = playlist_id(id).ok_or_else(|| Error::Api {
            status: 400,
            message: "Ungültige Spotify-Playlist-ID".into(),
        })?;
        let meta_url = format!("{}/playlists/{id}", self.api_url);
        let meta = match self.get_json(access_token, &meta_url).await {
            Err(Error::Api {
                status: 404,
                message,
            }) => {
                return Err(Error::Api {
                    status: 404,
                    message: format!(
                        "Playlist nicht gefunden ({message}) – Spotify-eigene und \
                         algorithmische Playlists gibt die Web-API nicht heraus"
                    ),
                })
            }
            r => r?,
        };
        let entry =
            parse::playlist_entry(&meta).ok_or_else(|| Error::Parse("Playlist ohne ID".into()))?;
        let summary = entry.summary;

        // Erste Seite steckt normalerweise im Playlist-Objekt (`items`, früher `tracks`)
        let (first, path) = match parse::playlist_track_page(&meta) {
            Some((key, page)) => (page.clone(), format!("/playlists/{id}/{key}")),
            None => self.first_items_page(access_token, &id).await?,
        };

        let mut tracks = Vec::new();
        self.follow_pages(access_token, first, &path, "Playlist-Titel", |page| {
            tracks.extend(parse::track_page_items(page));
            true
        })
        .await?;

        let mut summary = summary;
        if summary.track_count == 0 {
            summary.track_count = tracks.len().min(u32::MAX as usize) as u32;
        }
        Ok(SpPlaylist { summary, tracks })
    }

    /// Erste Titelseite über den eigenen Endpunkt: `/items` (seit 2026), bei `404` das ältere
    /// `/tracks`. `403` → verständliche Meldung (nur eigene/kollaborative Playlists).
    async fn first_items_page(&self, access_token: &str, id: &str) -> Result<(Value, String)> {
        let mut last = None;
        for key in ["items", "tracks"] {
            let path = format!("/playlists/{id}/{key}");
            let url = format!("{}{path}?offset=0&limit={MAX_PAGE_LIMIT}", self.api_url);
            match self.get_json(access_token, &url).await {
                Ok(v) => return Ok((v, path)),
                Err(Error::Api {
                    status: 404,
                    message,
                }) => {
                    last = Some(Error::Api {
                        status: 404,
                        message,
                    })
                }
                Err(Error::Api {
                    status: 403,
                    message,
                }) => {
                    return Err(Error::Api {
                        status: 403,
                        message: format!(
                            "Spotify gibt die Titel dieser Playlist nicht heraus ({message}) – \
                             Apps im Entwicklungsmodus sehen nur eigene und kollaborative \
                             Playlists. Abhilfe: in Spotify die Titel in eine eigene Playlist \
                             kopieren."
                        ),
                    })
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Parse("Playlist ohne Titelliste".into())))
    }

    // ------------------------------------------------------------ HTTP

    /// Anfrage senden; bei `429` (und `502`/`503`/`504`) bis zu [`MAX_RETRIES`]-mal nach
    /// `Retry-After` wiederholen, solange die Wartezeit kurz ist.
    async fn send<F>(&self, build: F) -> Result<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0;
        loop {
            let resp = build().send().await?;
            let status = resp.status();
            let retryable =
                status == StatusCode::TOO_MANY_REQUESTS || matches!(status.as_u16(), 502..=504);
            if !retryable || attempt >= MAX_RETRIES {
                return Ok(resp);
            }
            let wait = retry_after(&resp).unwrap_or(Duration::from_secs(1 << attempt));
            if wait > MAX_RETRY_WAIT {
                tracing::debug!("Spotify: {status}, Retry-After {wait:?} – nicht abgewartet");
                return Ok(resp);
            }
            tracing::debug!("Spotify: {status}, neuer Versuch in {wait:?}");
            drop(resp);
            tokio::time::sleep(wait).await;
            attempt += 1;
        }
    }

    /// GET mit Bearer-Token → JSON. `401` → [`Error::Unauthorized`], andere Fehler →
    /// [`Error::Api`] mit Spotifys Meldung.
    async fn get_json(&self, access_token: &str, url: &str) -> Result<Value> {
        if access_token.trim().is_empty() {
            return Err(Error::Unauthorized);
        }
        let resp = self
            .send(|| {
                self.http
                    .get(url)
                    .bearer_auth(access_token.trim())
                    .header(ACCEPT, "application/json")
            })
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(api_error(status, &body));
        }
        json_body(&body, url.strip_prefix(&self.api_url).unwrap_or(url))
    }

    /// Seiten ab `first` durchgehen, solange `on_page` `true` liefert und Spotify ein `next`
    /// nennt. `path` (z. B. "/me/tracks") baut die nächste Adresse selbst, falls `next` auf
    /// einen anderen Server zeigt – das Token geht nur an [`Spotify::api_url`].
    async fn follow_pages<F>(
        &self,
        access_token: &str,
        first: Value,
        path: &str,
        what: &str,
        mut on_page: F,
    ) -> Result<()>
    where
        F: FnMut(&Value) -> bool,
    {
        let mut page = first;
        let mut pages = 1;
        loop {
            expect_items(&page, what)?;
            let info = parse::page_info(&page);
            if !on_page(&page) || info.count == 0 || pages >= MAX_PAGES {
                return Ok(());
            }
            let Some(next) = self.next_url(&info, path) else {
                return Ok(());
            };
            page = self.get_json(access_token, &next).await?;
            pages += 1;
        }
    }

    fn next_url(&self, info: &parse::PageInfo, path: &str) -> Option<String> {
        let next = info.next.as_deref()?;
        if same_origin(next, &self.api_url) {
            return Some(next.to_string());
        }
        let limit = info
            .limit
            .filter(|&l| l > 0)
            .unwrap_or(info.count as u64)
            .clamp(1, MAX_PAGE_LIMIT as u64);
        let offset = info.offset.unwrap_or(0) + info.count.max(limit as usize) as u64;
        Some(format!(
            "{}{path}?offset={offset}&limit={limit}",
            self.api_url
        ))
    }
}

impl Default for Spotify {
    fn default() -> Self {
        Self::new()
    }
}

/// Spotify-Playlist-ID aus einer ID, URI (`spotify:playlist:…`, auch
/// `spotify:user:…:playlist:…`) oder einem Link (`https://open.spotify.com/playlist/…?si=…`,
/// auch mit Sprachpfad wie `/intl-de/`). `None`, wenn nichts Gültiges darin steht.
pub fn playlist_id(input: &str) -> Option<String> {
    let s = input.trim();
    let candidate = if let Some((_, rest)) = s.split_once(":playlist:") {
        rest
    } else if s.contains("://") || s.starts_with("open.spotify.com") {
        let (_, rest) = s.split_once("/playlist/")?;
        rest.split(['?', '#', '/']).next().unwrap_or("")
    } else {
        s
    };
    let id = candidate.trim();
    (!id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| id.to_string())
}

/// Seite muss eine `items`-Liste haben.
fn expect_items(page: &Value, what: &str) -> Result<()> {
    if page.get("items").is_some_and(Value::is_array) {
        Ok(())
    } else {
        Err(Error::Parse(format!("{what}: Seite ohne items")))
    }
}

fn totp_error(version: u32) -> Error {
    Error::Api {
        status: 503,
        message: format!(
            "Spotify nimmt den Webplayer-Schlüssel (Version {version}) nicht mehr an – \
             vermutlich hat Spotify ihn gerade gewechselt. Später erneut versuchen."
        ),
    }
}

fn json_body(body: &str, what: &str) -> Result<Value> {
    serde_json::from_str(body).map_err(|e| Error::Parse(format!("{what}: {e}")))
}

/// Fehlerantwort → [`Error`]: `401` → [`Error::Unauthorized`], sonst [`Error::Api`] mit
/// Spotifys Meldung (oder dem Statustext).
fn api_error(status: StatusCode, body: &str) -> Error {
    if status == StatusCode::UNAUTHORIZED {
        return Error::Unauthorized;
    }
    let message = parse::error_message(body).unwrap_or_else(|| {
        status
            .canonical_reason()
            .unwrap_or("unbekannter Fehler")
            .to_string()
    });
    Error::Api {
        status: status.as_u16(),
        message,
    }
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let value = resp.headers().get(RETRY_AFTER)?.to_str().ok()?.trim();
    value
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            value
                .parse::<f64>()
                .ok()
                .filter(|s| s.is_finite() && *s >= 0.0)
                .map(Duration::from_secs_f64)
        })
}

/// Gleiches Schema, gleicher Host, gleicher Port.
fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => {
            a.scheme() == b.scheme()
                && a.host_str() == b.host_str()
                && a.port_or_known_default() == b.port_or_known_default()
        }
        _ => false,
    }
}

fn is_loopback(u: &str) -> bool {
    url::Url::parse(u)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .is_some_and(|h| h == "127.0.0.1" || h == "[::1]" || h == "localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_needs_client_id() {
        let sp = Spotify::new();
        assert!(!sp.is_configured());
        assert!(matches!(sp.authorize_url(), Err(Error::NotConfigured)));
        sp.configure("  ", "http://127.0.0.1:8350/api/v1/spotify/callback");
        assert!(matches!(sp.authorize_url(), Err(Error::NotConfigured)));
        sp.configure("abc", "");
        assert!(sp.is_configured());
        assert!(matches!(sp.authorize_url(), Err(Error::NotConfigured)));
    }

    #[test]
    fn authorize_url_has_all_parameters() {
        let sp = Spotify::new();
        sp.configure(
            " 0123456789abcdef0123456789abcdef ",
            "http://127.0.0.1:8350/api/v1/spotify/callback",
        );
        let (url, pending) = sp.authorize_url().unwrap();
        assert!(
            url.starts_with("https://accounts.spotify.com/authorize?"),
            "{url}"
        );
        assert!(!url.contains('+'), "{url}");
        assert!(
            url.contains("scope=user-library-read%20playlist-read-private"),
            "{url}"
        );
        let parsed = url::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<String, String> =
            parsed.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "0123456789abcdef0123456789abcdef");
        assert_eq!(q["response_type"], "code");
        assert_eq!(
            q["redirect_uri"],
            "http://127.0.0.1:8350/api/v1/spotify/callback"
        );
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], auth::challenge(&pending.verifier));
        assert_eq!(q["scope"], SCOPES);
        assert_eq!(q["state"], pending.state);
        assert_eq!(pending.verifier.len(), 64);

        // Jede Anmeldung bekommt neue Werte
        let (_, again) = sp.authorize_url().unwrap();
        assert_ne!(again.state, pending.state);
        assert_ne!(again.verifier, pending.verifier);
    }

    #[test]
    fn playlist_ids() {
        let id = Some("37i9dQZF1DXcBWIGoYBM5M".to_string());
        assert_eq!(playlist_id("37i9dQZF1DXcBWIGoYBM5M"), id);
        assert_eq!(playlist_id(" spotify:playlist:37i9dQZF1DXcBWIGoYBM5M "), id);
        assert_eq!(
            playlist_id("spotify:user:spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
            id
        );
        assert_eq!(
            playlist_id("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc123"),
            id
        );
        assert_eq!(
            playlist_id("https://open.spotify.com/intl-de/playlist/37i9dQZF1DXcBWIGoYBM5M"),
            id
        );
        assert_eq!(
            playlist_id("open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M#x"),
            id
        );
        assert_eq!(playlist_id(""), None);
        assert_eq!(playlist_id("https://open.spotify.com/album/1234"), None);
        assert_eq!(playlist_id("../../me"), None);
        assert_eq!(playlist_id("abc?def"), None);
    }

    #[test]
    fn origins() {
        assert!(same_origin(
            "https://api.spotify.com/v1/me/tracks?offset=50",
            "https://api.spotify.com/v1"
        ));
        assert!(same_origin(
            "https://api.spotify.com:443/x",
            "https://api.spotify.com/v1"
        ));
        assert!(!same_origin(
            "https://evil.example/v1",
            "https://api.spotify.com/v1"
        ));
        assert!(!same_origin(
            "http://api.spotify.com/v1",
            "https://api.spotify.com/v1"
        ));
        assert!(!same_origin("kaputt", "https://api.spotify.com/v1"));
        assert!(is_loopback("http://127.0.0.1:1234"));
        assert!(!is_loopback("https://api.spotify.com/v1"));
    }

    #[test]
    fn next_url_stays_on_api_host() {
        let sp = Spotify::with_endpoints("http://127.0.0.1:9", "http://127.0.0.1:9/v1");
        let info = parse::PageInfo {
            total: Some(120),
            offset: Some(50),
            limit: Some(50),
            count: 50,
            next: Some("https://api.spotify.com/v1/me/tracks?offset=100&limit=50".into()),
        };
        assert_eq!(
            sp.next_url(&info, "/me/tracks").as_deref(),
            Some("http://127.0.0.1:9/v1/me/tracks?offset=100&limit=50")
        );
        let same = parse::PageInfo {
            next: Some("http://127.0.0.1:9/v1/me/tracks?offset=100&limit=50&market=CH".into()),
            ..info.clone()
        };
        assert_eq!(
            sp.next_url(&same, "/me/tracks").as_deref(),
            Some("http://127.0.0.1:9/v1/me/tracks?offset=100&limit=50&market=CH")
        );
        let end = parse::PageInfo { next: None, ..info };
        assert_eq!(sp.next_url(&end, "/me/tracks"), None);
    }

    /// Der Server startet die Anfragen in eigenen Tasks – die Futures müssen `Send` sein.
    #[test]
    fn futures_are_send() {
        fn assert_send<T: Send>(_: T) {}
        let sp = Spotify::new();
        assert_send(sp.exchange_code("c", "v"));
        assert_send(sp.refresh("r"));
        assert_send(sp.me("t"));
        assert_send(sp.liked("t", 0, 50));
        assert_send(sp.all_liked("t", 10));
        assert_send(sp.playlists("t"));
        assert_send(sp.playlist_entries("t"));
        assert_send(sp.playlist("t", "x"));
        fn assert_sync<T: Send + Sync>() {}
        assert_sync::<Spotify>();
    }

    #[test]
    fn tokens_expiry() {
        let t = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: auth::now() + 30,
            scope: String::new(),
        };
        assert!(t.expires_within(60));
        assert!(!t.expires_within(0));
    }

    #[test]
    fn api_errors() {
        assert!(matches!(
            api_error(StatusCode::UNAUTHORIZED, ""),
            Error::Unauthorized
        ));
        match api_error(
            StatusCode::FORBIDDEN,
            r#"{"error":{"status":403,"message":"Insufficient client scope"}}"#,
        ) {
            Error::Api { status, message } => {
                assert_eq!(status, 403);
                assert_eq!(message, "Insufficient client scope");
            }
            e => panic!("{e:?}"),
        }
        match api_error(StatusCode::BAD_GATEWAY, "<html>") {
            Error::Api { status, message } => {
                assert_eq!(status, 502);
                assert_eq!(message, "Bad Gateway");
            }
            e => panic!("{e:?}"),
        }
    }
}
