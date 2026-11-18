//! Client für YouTube Music über die InnerTube-API (`music.youtube.com/youtubei/v1`,
//! Client `WEB_REMIX`) – ohne Konto, ohne Schlüssel. Liefert die gemeinsamen Typen aus
//! `nrzmusic-types`. Abgespielt wird im Browser über den offiziellen YouTube-IFrame-Player;
//! dieser Client lädt keine Audiodaten.
//!
//! Endpunkte, Such-Parameter und die Wege durch die Antworten folgen dem quelloffenen
//! Projekt ytmusicapi. Die Antworten werden nachsichtig ausgewertet: Fehlt der Kern einer
//! Seite, gibt es [`Error::Parse`] (bzw. [`Error::NotFound`], wenn YouTube keinen Inhalt
//! liefert); einzelne unbekannte oder kaputte Einträge werden übersprungen.

mod cache;
mod json;
mod parse;

use std::sync::RwLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, COOKIE, ORIGIN, REFERER};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::cache::{Cache, Cached};
use crate::json::JsonExt;

pub use nrzmusic_types::{YtAlbum, YtArtist, YtPlaylist, YtSearchResults, YtShelf, YtTrack};
pub use nrzmusic_types::{
    YtAlbumRef, YtAlbumSummary, YtArtistRef, YtArtistSummary, YtItem, YtPlaylistSummary,
};
pub use parse::common::{parse_duration_ms, upscale_thumbnail};

const API_BASE: &str = "https://music.youtube.com/youtubei/v1";
const YTM_ORIGIN: &str = "https://music.youtube.com";
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
/// Gegen die Einwilligungsseite in der EU (wie ytmusicapi und yt-dlp)
const CONSENT_COOKIE: &str = "SOCS=CAI";

/// Höchstens so viele Titel lädt [`YtMusic::playlist`] (reicht für Importe).
pub const MAX_PLAYLIST_TRACKS: usize = 2000;
/// Ziel-Anzahl Treffer einer gefilterten Suche (mehrere Seiten).
const FILTERED_SEARCH_RESULTS: usize = 40;
const MAX_SEARCH_PAGES: usize = 3;
const MAX_PLAYLIST_PAGES: usize = 40;
/// Die Startseite wird so lange fortgesetzt, bis mindestens so viele Regale da sind.
const HOME_MIN_SHELVES: usize = 8;
const MAX_HOME_PAGES: usize = 4;
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_MAX_ENTRIES: usize = 128;
/// Grössere Playlists (Importe) werden nicht zwischengespeichert – zu viel Speicher.
const CACHE_MAX_PLAYLIST_TRACKS: usize = 500;
/// Nach dieser Zeit holt sich der Client eine neue Visitor-ID.
const VISITOR_MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("YouTube Music nicht erreichbar: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Unerwartete Antwort von YouTube Music: {0}")]
    Parse(String),
    #[error("Nicht gefunden")]
    NotFound,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Sprache (`hl`) und Region (`gl`) der Anfragen.
#[derive(Clone, Debug, PartialEq)]
pub struct Locale {
    pub hl: String,
    pub gl: String,
}

impl Default for Locale {
    fn default() -> Self {
        Locale {
            hl: "de".into(),
            gl: "CH".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchFilter {
    Songs,
    Videos,
    Albums,
    Artists,
    Playlists,
}

impl SearchFilter {
    /// "songs", "videos", "albums", "artists", "playlists"
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "songs" => Some(Self::Songs),
            "videos" => Some(Self::Videos),
            "albums" => Some(Self::Albums),
            "artists" => Some(Self::Artists),
            "playlists" => Some(Self::Playlists),
            _ => None,
        }
    }
}

/// Ein Client für den ganzen Server (intern ein `reqwest::Client`, billig zu teilen).
pub struct YtMusic {
    http: reqwest::Client,
    locale: RwLock<Locale>,
    /// `visitorData` aus der ersten Antwort (mit Zeitpunkt), als `X-Goog-Visitor-Id`
    /// wiederverwendet und nach [`VISITOR_MAX_AGE`] erneuert
    visitor: RwLock<Option<(String, Instant)>>,
    cache: Cache,
}

impl YtMusic {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        YtMusic {
            http,
            locale: RwLock::new(Locale::default()),
            visitor: RwLock::new(None),
            cache: Cache::new(CACHE_TTL, CACHE_MAX_ENTRIES),
        }
    }

    /// Sprache und Region für alle folgenden Anfragen (leere Angaben → Standard "de"/"CH").
    pub fn set_locale(&self, hl: &str, gl: &str) {
        let default = Locale::default();
        let hl = if hl.trim().is_empty() {
            default.hl
        } else {
            hl.trim().to_string()
        };
        let gl = if gl.trim().is_empty() {
            default.gl
        } else {
            gl.trim().to_uppercase()
        };
        *self.locale.write().unwrap_or_else(|e| e.into_inner()) = Locale { hl, gl };
    }

    /// Aktuelle Sprache und Region.
    pub fn locale(&self) -> Locale {
        self.locale
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Leert den Zwischenspeicher (Alben, Künstler, Playlists, Startseite).
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Suche. Ohne Filter: von jeder Kategorie die Treffer der gemischten Ergebnisseite
    /// (inklusive Top-Treffer); mit Filter: nur diese Kategorie, dafür mehr Treffer.
    pub async fn search(
        &self,
        query: &str,
        filter: Option<SearchFilter>,
    ) -> Result<YtSearchResults> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(YtSearchResults::default());
        }
        let mut body = json!({ "query": query });
        if let Some(f) = filter {
            body["params"] = json!(parse::search::filter_params(f));
        }
        let resp = self.post("search", body, None).await?;
        let mut collector = parse::search::SearchCollector::new(filter);
        let mut token = parse::search::parse_search(&resp, &mut collector)?;

        if filter.is_some() {
            let visitor = visitor_of(&resp);
            let mut pages = 0;
            while let Some(t) = token.take() {
                if collector.len() >= FILTERED_SEARCH_RESULTS || pages >= MAX_SEARCH_PAGES {
                    break;
                }
                pages += 1;
                let before = collector.len();
                match self
                    .post("search", json!({ "continuation": t }), visitor.as_deref())
                    .await
                {
                    Ok(resp) => {
                        token = parse::search::parse_search_continuation(&resp, &mut collector)
                    }
                    Err(e) => {
                        tracing::debug!("YouTube-Suche: weitere Seite fehlgeschlagen: {e}");
                        break;
                    }
                }
                if collector.len() == before {
                    break;
                }
            }
        }
        Ok(collector.results)
    }

    /// Suchvorschläge als einfache Texte.
    pub async fn suggestions(&self, query: &str) -> Result<Vec<String>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let resp = self
            .post(
                "music/get_search_suggestions",
                json!({ "input": query }),
                None,
            )
            .await?;
        Ok(parse::search::parse_suggestions(&resp))
    }

    /// Album (`MPREb_…`) mit allen Titeln.
    pub async fn album(&self, browse_id: &str) -> Result<YtAlbum> {
        let id = browse_id.trim();
        if !is_valid_id(id) {
            return Err(Error::NotFound);
        }
        let key = self.cache_key("album", id);
        if let Some(Cached::Album(album)) = self.cache.get(&key) {
            return Ok(album);
        }
        let resp = self.post("browse", json!({ "browseId": id }), None).await?;
        let album = parse::album::parse_album(&resp, id)?;
        self.cache.put(key, Cached::Album(album.clone()));
        Ok(album)
    }

    /// Playlist mit bis zu [`MAX_PLAYLIST_TRACKS`] Titeln. Nimmt die ID mit oder ohne
    /// "VL" (auch einen Link). Mixe ("RD…") ohne eigene Seite kommen aus der Warteschlange.
    pub async fn playlist(&self, playlist_id: &str) -> Result<YtPlaylist> {
        let id = normalize_playlist_id(playlist_id).ok_or(Error::NotFound)?;
        let key = self.cache_key("playlist", &id);
        if let Some(Cached::Playlist(playlist)) = self.cache.get(&key) {
            return Ok(playlist);
        }
        let playlist = match self.fetch_playlist(&id).await {
            Err(Error::NotFound) if id.starts_with("RD") => self.fetch_mix(&id).await?,
            other => other?,
        };
        if playlist.tracks.len() <= CACHE_MAX_PLAYLIST_TRACKS {
            self.cache.put(key, Cached::Playlist(playlist.clone()));
        }
        Ok(playlist)
    }

    async fn fetch_playlist(&self, id: &str) -> Result<YtPlaylist> {
        let resp = self
            .post("browse", json!({ "browseId": format!("VL{id}") }), None)
            .await?;
        let (mut playlist, mut token) = parse::playlist::parse_playlist(&resp, id)?;
        let visitor = visitor_of(&resp);
        let mut pages = 0;
        while let Some(t) = token.take() {
            if playlist.tracks.len() >= MAX_PLAYLIST_TRACKS || pages >= MAX_PLAYLIST_PAGES {
                break;
            }
            pages += 1;
            let body = json!({ "continuation": t });
            // Ein zweiter Versuch, damit ein Import nicht wegen eines Aussetzers halb bleibt
            let resp = match self.post("browse", body.clone(), visitor.as_deref()).await {
                Ok(r) => r,
                Err(_) => match self.post("browse", body, visitor.as_deref()).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("YouTube-Playlist {id}: weitere Titel nicht geladen: {e}");
                        break;
                    }
                },
            };
            let (more, next) = parse::playlist::parse_playlist_continuation(&resp);
            if more.is_empty() {
                break;
            }
            playlist.tracks.extend(more);
            token = next;
        }
        playlist.tracks.truncate(MAX_PLAYLIST_TRACKS);
        Ok(playlist)
    }

    /// Mix ohne Playlist-Seite (z. B. "RDAMVM…", "RDEM…") über die Warteschlange.
    async fn fetch_mix(&self, id: &str) -> Result<YtPlaylist> {
        let mut body = json!({
            "playlistId": id,
            "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true,
        });
        if let Some(video_id) = id.strip_prefix("RDAMVM").filter(|v| is_valid_id(v)) {
            body["videoId"] = json!(video_id);
        }
        let resp = self.post("next", body, None).await?;
        let (tracks, title) = parse::watch::parse_watch(&resp, None)?;
        if tracks.is_empty() {
            return Err(Error::NotFound);
        }
        let title = title.unwrap_or_else(|| format!("Mix – {}", tracks[0].title));
        Ok(YtPlaylist {
            summary: YtPlaylistSummary {
                playlist_id: id.to_string(),
                title,
                author: Some("YouTube Music".into()),
                track_count: None,
                thumbnail: tracks[0].thumbnail.clone(),
            },
            description: None,
            tracks,
        })
    }

    /// Künstlerseite (`UC…`): Top-Songs, Alben, Singles, Beschreibung, Abonnenten.
    pub async fn artist(&self, channel_id: &str) -> Result<YtArtist> {
        let id = channel_id.trim();
        let id = id.strip_prefix("MPLA").unwrap_or(id);
        if !is_valid_id(id) {
            return Err(Error::NotFound);
        }
        let key = self.cache_key("artist", id);
        if let Some(Cached::Artist(artist)) = self.cache.get(&key) {
            return Ok(artist);
        }
        let resp = self.post("browse", json!({ "browseId": id }), None).await?;
        let artist = parse::artist::parse_artist(&resp, id)?;
        self.cache.put(key, Cached::Artist(artist.clone()));
        Ok(artist)
    }

    /// "Radio" / "Als Nächstes" zu einem Titel (ohne den Titel selbst).
    pub async fn radio(&self, video_id: &str) -> Result<Vec<YtTrack>> {
        let id = video_id.trim();
        if !is_valid_id(id) {
            return Err(Error::NotFound);
        }
        let body = json!({
            "videoId": id,
            "playlistId": format!("RDAMVM{id}"),
            "params": "wAEB",
            "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true,
            "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
        });
        let resp = self.post("next", body, None).await?;
        Ok(parse::watch::parse_watch(&resp, Some(id))?.0)
    }

    /// Regale der Startseite (ohne Konto: Charts, Stimmungen, Neuerscheinungen …).
    pub async fn home(&self) -> Result<Vec<YtShelf>> {
        let key = self.cache_key("home", "");
        if let Some(Cached::Home(shelves)) = self.cache.get(&key) {
            return Ok(shelves);
        }
        let resp = self
            .post("browse", json!({ "browseId": "FEmusic_home" }), None)
            .await?;
        let (mut shelves, mut token) = parse::home::parse_home(&resp)?;
        let visitor = visitor_of(&resp);
        let mut pages = 0;
        while let Some(t) = token.take() {
            if shelves.len() >= HOME_MIN_SHELVES || pages >= MAX_HOME_PAGES {
                break;
            }
            pages += 1;
            match self
                .post("browse", json!({ "continuation": t }), visitor.as_deref())
                .await
            {
                Ok(resp) => {
                    let (more, next) = parse::home::parse_home_continuation(&resp);
                    shelves.extend(more);
                    token = next;
                }
                Err(e) => {
                    tracing::debug!("YouTube-Startseite: weitere Regale fehlgeschlagen: {e}");
                    break;
                }
            }
        }
        self.cache.put(key, Cached::Home(shelves.clone()));
        Ok(shelves)
    }

    fn cache_key(&self, kind: &str, id: &str) -> String {
        let l = self.locale();
        format!("{kind}:{}:{}:{id}", l.hl, l.gl)
    }

    /// POST an `youtubei/v1/<endpoint>` mit WEB_REMIX-Kontext. `visitor` ersetzt die
    /// gespeicherte Visitor-ID (Fortsetzungen müssen zur ersten Antwort passen).
    async fn post(&self, endpoint: &str, mut body: Value, visitor: Option<&str>) -> Result<Value> {
        let locale = self.locale();
        let version = client_version();
        let visitor = match visitor {
            Some(v) => Some(v.to_string()),
            None => self
                .visitor
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .filter(|(_, at)| at.elapsed() < VISITOR_MAX_AGE)
                .map(|(v, _)| v.clone()),
        };

        let mut client = json!({
            "clientName": "WEB_REMIX",
            "clientVersion": version,
            "hl": locale.hl,
            "gl": locale.gl,
        });
        if let Some(v) = &visitor {
            client["visitorData"] = json!(v);
        }
        if let Some(map) = body.as_object_mut() {
            map.insert("context".into(), json!({ "client": client, "user": {} }));
        }

        let mut request = self
            .http
            .post(format!("{API_BASE}/{endpoint}?prettyPrint=false"))
            .header(ACCEPT, "*/*")
            .header(
                ACCEPT_LANGUAGE,
                format!("{},en;q=0.8", locale.hl.replace('_', "-")),
            )
            .header(ORIGIN, YTM_ORIGIN)
            .header(REFERER, format!("{YTM_ORIGIN}/"))
            .header(COOKIE, CONSENT_COOKIE)
            .header("X-YouTube-Client-Name", "67")
            .header("X-YouTube-Client-Version", &version)
            .json(&body);
        if let Some(v) = &visitor {
            request = request.header("X-Goog-Visitor-Id", v);
        }

        let response = request.send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(Error::NotFound);
        }
        let bytes = response.error_for_status()?.bytes().await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Parse(format!("keine gültige JSON-Antwort ({e})")))?;

        if let Some(v) = value
            .str_at("/responseContext/visitorData")
            .filter(|v| !v.is_empty())
        {
            let mut stored = self.visitor.write().unwrap_or_else(|e| e.into_inner());
            if stored
                .as_ref()
                .is_none_or(|(_, at)| at.elapsed() >= VISITOR_MAX_AGE)
            {
                *stored = Some((v.to_string(), Instant::now()));
            }
        }
        Ok(value)
    }
}

impl Default for YtMusic {
    fn default() -> Self {
        Self::new()
    }
}

fn visitor_of(resp: &Value) -> Option<String> {
    resp.str_at("/responseContext/visitorData")
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// `clientVersion` wie ytmusicapi: `1.YYYYMMDD.01.00` (heutiges Datum, UTC).
fn client_version() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(days as i64);
    format!("1.{y:04}{m:02}{d:02}.01.00")
}

/// Tage seit 1970-01-01 → (Jahr, Monat, Tag) im gregorianischen Kalender.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Nur Zeichen, die in IDs von YouTube vorkommen.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Playlist-ID aus Link oder ID; "VL" wird entfernt.
fn normalize_playlist_id(input: &str) -> Option<String> {
    let input = input.trim();
    if input.contains(['/', '?', '=', '.']) {
        return playlist_id_from_url(input);
    }
    let id = input.strip_prefix("VL").unwrap_or(input);
    is_valid_id(id).then(|| id.to_string())
}

/// Playlist-ID aus einem Link von YouTube oder YouTube Music (`list=`), oder die ID selbst.
///
/// Erkennt Links von music.youtube.com, (www./m.)youtube.com und youtu.be sowie blosse IDs,
/// die mit `PL`, `OLAK5uy_`, `RD` oder `VL` beginnen (`VL` wird entfernt).
pub fn playlist_id_from_url(url: &str) -> Option<String> {
    let input = url.trim();
    if input.is_empty() {
        return None;
    }

    // Blosse ID
    if !input.contains(['/', '?', '=', '.', ' ']) {
        let id = input.strip_prefix("VL").unwrap_or(input);
        let known =
            ["PL", "OLAK5uy_", "RD"].iter().any(|p| id.starts_with(p)) || input.starts_with("VL");
        return (known && is_valid_id(id) && id.len() >= 10).then(|| id.to_string());
    }

    let with_scheme = if input.starts_with("http://") || input.starts_with("https://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let parsed = url::Url::parse(&with_scheme).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if !matches!(
        host,
        "music.youtube.com" | "youtube.com" | "m.youtube.com" | "youtu.be" | "youtube-nocookie.com"
    ) {
        return None;
    }
    let list = parsed
        .query_pairs()
        .find(|(k, _)| k == "list")
        .map(|(_, v)| v.into_owned())?;
    let list = list.trim();
    let id = list.strip_prefix("VL").unwrap_or(list);
    // Persönliche Listen ("LL", "WL", "LM") sind ohne Konto nicht lesbar
    (is_valid_id(id) && id.len() >= 10).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_version_format() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_757), (2026, 10, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        let v = client_version();
        assert!(
            v.starts_with("1.20") && v.ends_with(".01.00") && v.len() == 16,
            "{v}"
        );
    }

    #[test]
    fn playlist_ids() {
        let pl = "PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ";
        assert_eq!(
            playlist_id_from_url(&format!("https://music.youtube.com/playlist?list={pl}"))
                .as_deref(),
            Some(pl)
        );
        assert_eq!(
            playlist_id_from_url(&format!(
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list={pl}&index=3"
            ))
            .as_deref(),
            Some(pl)
        );
        assert_eq!(
            playlist_id_from_url(&format!("https://youtu.be/dQw4w9WgXcQ?list={pl}")).as_deref(),
            Some(pl)
        );
        assert_eq!(
            playlist_id_from_url(&format!("m.youtube.com/playlist?list=VL{pl}")).as_deref(),
            Some(pl)
        );
        assert_eq!(
            playlist_id_from_url(&format!("  {pl}  ")).as_deref(),
            Some(pl)
        );
        assert_eq!(
            playlist_id_from_url(&format!("VL{pl}")).as_deref(),
            Some(pl)
        );
        let olak = "OLAK5uy_kNhM2yaBTOVwrcZJepB1C9P3-n5_Sfy5c";
        assert_eq!(playlist_id_from_url(olak).as_deref(), Some(olak));
        assert_eq!(
            playlist_id_from_url("RDCLAK5uy_kf6c8Ga41e1bLKYCaulkiZ1NIVYJnr3Jk").as_deref(),
            Some("RDCLAK5uy_kf6c8Ga41e1bLKYCaulkiZ1NIVYJnr3Jk")
        );
        assert_eq!(
            playlist_id_from_url("RDAMVMiRHo9cz2wFc").as_deref(),
            Some("RDAMVMiRHo9cz2wFc")
        );

        assert_eq!(playlist_id_from_url(""), None);
        assert_eq!(playlist_id_from_url("hello world"), None);
        assert_eq!(
            playlist_id_from_url("dQw4w9WgXcQ"),
            None,
            "Video-ID ist keine Playlist"
        );
        assert_eq!(
            playlist_id_from_url("https://music.youtube.com/watch?v=dQw4w9WgXcQ"),
            None
        );
        assert_eq!(
            playlist_id_from_url(&format!("https://example.com/playlist?list={pl}")),
            None
        );
        assert_eq!(
            playlist_id_from_url("https://www.youtube.com/playlist?list=LL"),
            None
        );
        assert_eq!(
            playlist_id_from_url("https://www.youtube.com/playlist?list=PL<script>"),
            None
        );
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_playlist_id("VLPL123").as_deref(), Some("PL123"));
        assert_eq!(normalize_playlist_id("PL123").as_deref(), Some("PL123"));
        assert_eq!(normalize_playlist_id("bad id"), None);
        assert_eq!(
            normalize_playlist_id(
                "https://music.youtube.com/playlist?list=PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ"
            )
            .as_deref(),
            Some("PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ")
        );
    }

    #[test]
    fn locale_switch() {
        let yt = YtMusic::new();
        assert_eq!(yt.locale(), Locale::default());
        yt.set_locale("en", "us");
        assert_eq!(
            yt.locale(),
            Locale {
                hl: "en".into(),
                gl: "US".into()
            }
        );
        assert!(yt.cache_key("album", "x").contains(":en:US:"));
        yt.set_locale("", "");
        assert_eq!(yt.locale(), Locale::default());
    }

    #[test]
    fn usable_from_async_server() {
        // Rocket teilt den Client zwischen Threads und verlangt Send-Futures
        fn send_sync<T: Send + Sync>(_: &T) {}
        fn send<T: Send>(_: T) {}
        let yt = YtMusic::new();
        send_sync(&yt);
        send(yt.search("x", None));
        send(yt.suggestions("x"));
        send(yt.album("x"));
        send(yt.playlist("x"));
        send(yt.artist("x"));
        send(yt.radio("x"));
        send(yt.home());
    }

    #[test]
    fn filters() {
        assert_eq!(SearchFilter::parse("songs"), Some(SearchFilter::Songs));
        assert_eq!(SearchFilter::parse("podcasts"), None);
    }
}
