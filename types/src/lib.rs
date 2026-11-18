//! Gemeinsame Typen der JSON-API von nrzMusic. Der Server (Rocket), die Oberfläche
//! (Dioxus/WebAssembly) und der YouTube-Music-Client benutzen genau diese Strukturen –
//! eine Änderung hier ändert den Vertrag für alle drei.
//!
//! Zeitpunkte sind Unix-Sekunden (`i64`), Dauern Millisekunden (`u64`).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- Bibliothek (lokal)

/// Ein Titel aus dem Musikordner.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Track {
    pub id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    /// Stabile ID des Albums (Hash aus Album-Künstler und Albumtitel)
    pub album_id: String,
    /// Stabile ID des Album-Künstlers (Hash aus dem Namen)
    pub artist_id: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub duration_ms: u64,
    pub bitrate_kbps: Option<u32>,
    /// Dateiendung in Kleinbuchstaben: "mp3", "flac", "m4a", "ogg", "opus", "wav"
    pub format: String,
    pub size: u64,
    /// Pfad relativ zum Musikordner
    pub path: String,
    /// Cover-ID → `/api/v1/covers/<id>`
    pub cover: Option<String>,
    pub added_at: i64,
    pub play_count: u32,
    pub last_played: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub year: Option<i32>,
    pub track_count: u32,
    pub duration_ms: u64,
    pub cover: Option<String>,
    /// Neuester `added_at` seiner Titel
    pub added_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AlbumDetail {
    pub album: Album,
    /// Sortiert nach Disc, Titelnummer, Titel
    pub tracks: Vec<Track>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub album_count: u32,
    pub track_count: u32,
    /// Cover eines seiner Alben
    pub cover: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ArtistDetail {
    pub artist: Artist,
    /// Neueste zuerst
    pub albums: Vec<Album>,
    /// Alle Titel, meistgespielte zuerst
    pub tracks: Vec<Track>,
}

/// Eine Seite einer langen Liste (`?offset=&limit=`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

/// Lokale Suche über Titel, Alben und Künstler (`GET /api/v1/search?q=`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LibrarySearch {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub struct ScanProgress {
    pub done: u64,
    pub total: u64,
}

/// `GET /api/v1/status`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LibraryStatus {
    pub version: String,
    /// Musikordner, wie ihn der Server sieht (z. B. "/music")
    pub music_dir: String,
    pub tracks: u64,
    pub albums: u64,
    pub artists: u64,
    pub total_duration_ms: u64,
    pub total_size: u64,
    pub scanning: bool,
    pub scan_progress: Option<ScanProgress>,
    pub last_scan: Option<i64>,
    /// Der Ordner wird live überwacht (sonst nur periodischer Scan)
    pub watching: bool,
    pub youtube_enabled: bool,
}

/// `GET /api/v1/home` – Startseite.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct HomeData {
    pub recently_added: Vec<Album>,
    pub recently_played: Vec<PlayableTrack>,
    pub most_played: Vec<Track>,
    pub random_albums: Vec<Album>,
}

/// Antwort auf `POST /api/v1/upload` (multipart, Feld `files`, mehrfach).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct UploadResult {
    pub imported: Vec<Track>,
    pub skipped: Vec<UploadSkipped>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UploadSkipped {
    pub name: String,
    pub reason: String,
}

// ---------------------------------------------------------------- Lokal + YouTube gemischt

/// Alles, was der Player abspielen kann: ein lokaler Titel oder ein Titel von YouTube Music.
/// JSON: `{"source":"local", …Track}` bzw. `{"source":"youtube", …YtTrack}`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum PlayableTrack {
    Local(Track),
    Youtube(YtTrack),
}

/// Verweis auf einen Titel, z. B. zum Hinzufügen zu Playlist oder Favoriten.
/// YouTube-Titel bringen ihre Metadaten mit – der Server speichert sie.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum TrackRef {
    Local { id: i64 },
    Youtube(YtTrack),
}

impl PlayableTrack {
    /// Eindeutiger Schlüssel: `local:<id>` oder `yt:<videoId>` (auch in URLs benutzt).
    pub fn key(&self) -> String {
        match self {
            PlayableTrack::Local(t) => format!("local:{}", t.id),
            PlayableTrack::Youtube(t) => format!("yt:{}", t.video_id),
        }
    }

    pub fn title(&self) -> &str {
        match self {
            PlayableTrack::Local(t) => &t.title,
            PlayableTrack::Youtube(t) => &t.title,
        }
    }

    /// Künstler als eine Zeile ("A, B & C" bei YouTube).
    pub fn artist(&self) -> String {
        match self {
            PlayableTrack::Local(t) => t.artist.clone(),
            PlayableTrack::Youtube(t) => t.artist_line(),
        }
    }

    pub fn album(&self) -> Option<&str> {
        match self {
            PlayableTrack::Local(t) => Some(&t.album).filter(|a| !a.is_empty()).map(|a| a.as_str()),
            PlayableTrack::Youtube(t) => t.album.as_ref().map(|a| a.title.as_str()),
        }
    }

    pub fn duration_ms(&self) -> Option<u64> {
        match self {
            PlayableTrack::Local(t) => Some(t.duration_ms),
            PlayableTrack::Youtube(t) => t.duration_ms,
        }
    }

    /// Bild-URL: lokales Cover (vom Server) oder YouTube-Vorschaubild.
    /// `size` nur für lokale Cover (`?size=`), z. B. 96, 300 oder 600.
    pub fn artwork_url(&self, size: u32) -> Option<String> {
        match self {
            PlayableTrack::Local(t) => t.cover.as_ref().map(|c| cover_url(c, size)),
            PlayableTrack::Youtube(t) => t.thumbnail.clone(),
        }
    }

    pub fn to_ref(&self) -> TrackRef {
        match self {
            PlayableTrack::Local(t) => TrackRef::Local { id: t.id },
            PlayableTrack::Youtube(t) => TrackRef::Youtube(t.clone()),
        }
    }
}

impl TrackRef {
    pub fn key(&self) -> String {
        match self {
            TrackRef::Local { id } => format!("local:{id}"),
            TrackRef::Youtube(t) => format!("yt:{}", t.video_id),
        }
    }
}

/// URL eines lokalen Covers in einer bestimmten Kantenlänge (Pixel).
pub fn cover_url(cover: &str, size: u32) -> String {
    format!("/api/v1/covers/{cover}?size={size}")
}

/// `PUT /api/v1/favorites` – Body: [`TrackRef`]. `DELETE /api/v1/favorites/<key>`.
/// `GET /api/v1/favorites` → `Vec<PlayableTrack>` (neueste zuerst).
/// `GET /api/v1/favorites/keys` → `Vec<String>` (alle Schlüssel, für Herz-Symbole).

// ---------------------------------------------------------------- Playlists

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlaylistSummary {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub item_count: u32,
    /// Summe der bekannten Dauern
    pub duration_ms: u64,
    /// Bis zu vier Bild-URLs (fertig, mit `?size=300` bei lokalen Covern) für die Collage
    pub artwork: Vec<String>,
    /// Aus YouTube Music importiert (Playlist-ID dort)
    pub youtube_playlist_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Playlist {
    pub summary: PlaylistSummary,
    /// In Reihenfolge; Position = Index
    pub items: Vec<PlayableTrack>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NewPlaylist {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Gleich mit Inhalt anlegen (z. B. "Warteschlange als Playlist speichern")
    #[serde(default)]
    pub items: Vec<TrackRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PlaylistUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// `POST /api/v1/playlists/<id>/items` – hängt an.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AddItems {
    pub items: Vec<TrackRef>,
}

/// `PUT /api/v1/playlists/<id>/order` – neue Reihenfolge als Liste der alten Positionen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Reorder {
    pub order: Vec<u32>,
}

/// `POST /api/v1/yt/import` – YouTube-(Music-)Playlist-Link als nrzMusic-Playlist übernehmen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ImportRequest {
    pub url: String,
}

// ---------------------------------------------------------------- Einstellungen, Ereignisse

/// `GET/PUT /api/v1/settings` – fehlende Felder (ältere Clients, alte Datenbank) nehmen den
/// Standardwert.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub youtube_enabled: bool,
    /// Sprache für YouTube Music (`hl`), z. B. "de"
    pub youtube_language: String,
    /// Region für YouTube Music (`gl`), z. B. "CH"
    pub youtube_region: String,
    /// YouTube-Titel mit yt-dlp in den Musikordner laden
    pub downloads_enabled: bool,
    /// Zielformat der Downloads: "mp3" (VBR V0), "m4a" (AAC, ohne Umwandlung), "opus" (ohne Umwandlung)
    pub download_format: String,
    /// yt-dlp beim Start und täglich selbst aktualisieren
    pub ytdlp_auto_update: bool,
    /// Client-ID der eigenen Spotify-App (developer.spotify.com), leer = Spotify aus
    pub spotify_client_id: String,
    /// Redirect-URI, wie in der Spotify-App eingetragen; leer = Standard des Servers
    /// (`http://127.0.0.1:<port>/api/v1/spotify/callback`, siehe `SpotifyStatus.redirect_uri`)
    pub spotify_redirect_uri: String,
    /// Neue Konten dürfen sich selbst registrieren
    #[serde(default = "default_true")]
    pub registration_open: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            youtube_enabled: true,
            youtube_language: "de".into(),
            youtube_region: "CH".into(),
            downloads_enabled: true,
            download_format: "mp3".into(),
            ytdlp_auto_update: true,
            spotify_client_id: String::new(),
            spotify_redirect_uri: String::new(),
            registration_open: true,
        }
    }
}

/// Server-Sent Events unter `GET /api/v1/events` (`data:` = dieses JSON).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    ScanStarted,
    ScanProgress(ScanProgress),
    ScanFinished { added: u64, updated: u64, removed: u64 },
    /// Bibliothek hat sich geändert (Datei abgelegt, gelöscht, Upload) – Listen neu laden
    LibraryChanged { tracks: u64 },
    PlaylistsChanged,
    FavoritesChanged,
    /// Ein Download wurde angelegt, macht Fortschritt (gedrosselt), ist fertig oder fehlgeschlagen
    DownloadUpdate(Box<DownloadJob>),
    /// Spotify verbunden oder getrennt – `GET /spotify/status` neu laden
    SpotifyChanged,
    /// Fortschritt einer Spotify-Aufgabe (Abgleich, Import, "alle herunterladen")
    SpotifyProgress { task: String, done: u64, total: u64 },
}

/// Fehlerantwort jeder Route: `{"error": "…"}` mit passendem HTTP-Status.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ApiError {
    pub error: String,
}

// ---------------------------------------------------------------- YouTube Music

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct YtArtistRef {
    pub name: String,
    /// Kanal-/Browse-ID ("UC…"), fehlt bei reinen Textangaben
    pub id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct YtAlbumRef {
    pub title: String,
    /// Browse-ID ("MPREb_…")
    pub id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtTrack {
    pub video_id: String,
    pub title: String,
    pub artists: Vec<YtArtistRef>,
    pub album: Option<YtAlbumRef>,
    pub duration_ms: Option<u64>,
    /// Quadratisches Vorschaubild, möglichst gross (≥ 400 px)
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub explicit: bool,
    /// Musikvideo statt Audio-Titel
    #[serde(default)]
    pub is_video: bool,
}

impl YtTrack {
    /// "A, B & C"
    pub fn artist_line(&self) -> String {
        let names: Vec<&str> = self.artists.iter().map(|a| a.name.as_str()).collect();
        match names.len() {
            0 => String::new(),
            1 => names[0].to_string(),
            n => format!("{} & {}", names[..n - 1].join(", "), names[n - 1]),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtAlbumSummary {
    pub browse_id: String,
    pub title: String,
    pub artists: Vec<YtArtistRef>,
    pub year: Option<String>,
    /// "Album", "Single", "EP" – wie YouTube Music es anzeigt
    pub kind: String,
    pub thumbnail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtArtistSummary {
    pub browse_id: String,
    pub name: String,
    /// z. B. "1,2 Mio. Abonnenten"
    pub subscribers: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtPlaylistSummary {
    /// Playlist-ID ohne "VL"-Präfix (z. B. "PL…", "OLAK5uy_…", "RDCLAK…")
    pub playlist_id: String,
    pub title: String,
    pub author: Option<String>,
    /// Wie angezeigt, z. B. "42 Titel"
    pub track_count: Option<String>,
    pub thumbnail: Option<String>,
}

/// `GET /api/v1/yt/search?q=&filter=` (`filter`: songs, videos, albums, artists, playlists;
/// ohne Filter: von allem die ersten Treffer).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct YtSearchResults {
    pub songs: Vec<YtTrack>,
    pub videos: Vec<YtTrack>,
    pub albums: Vec<YtAlbumSummary>,
    pub artists: Vec<YtArtistSummary>,
    pub playlists: Vec<YtPlaylistSummary>,
}

/// `GET /api/v1/yt/album/<browse_id>`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtAlbum {
    pub summary: YtAlbumSummary,
    pub description: Option<String>,
    pub tracks: Vec<YtTrack>,
    /// Audio-Playlist des Albums ("OLAK5uy_…"), falls bekannt
    pub playlist_id: Option<String>,
}

/// `GET /api/v1/yt/playlist/<playlist_id>`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtPlaylist {
    pub summary: YtPlaylistSummary,
    pub description: Option<String>,
    pub tracks: Vec<YtTrack>,
}

/// `GET /api/v1/yt/artist/<channel_id>`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtArtist {
    pub summary: YtArtistSummary,
    pub description: Option<String>,
    pub top_songs: Vec<YtTrack>,
    pub albums: Vec<YtAlbumSummary>,
    pub singles: Vec<YtAlbumSummary>,
    /// Playlist mit allen Songs des Künstlers, falls angeboten
    pub songs_playlist_id: Option<String>,
}

/// Ein Eintrag in einem Regal der YouTube-Music-Startseite.
/// Markiert über `type` – `kind` gehört schon `YtAlbumSummary` ("Album", "Single").
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum YtItem {
    Track(YtTrack),
    Album(YtAlbumSummary),
    Artist(YtArtistSummary),
    Playlist(YtPlaylistSummary),
}

/// `GET /api/v1/yt/home` → `Vec<YtShelf>` (leer, wenn YouTube nichts liefert)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct YtShelf {
    pub title: String,
    pub items: Vec<YtItem>,
}

// ---------------------------------------------------------------- Downloads (yt-dlp)

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

/// Ein YouTube-Titel, der mit yt-dlp in den Musikordner geladen wird.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DownloadJob {
    pub id: i64,
    pub track: YtTrack,
    pub status: DownloadStatus,
    /// 0.0 – 1.0 (Download und Umwandlung)
    pub progress: f32,
    /// Lokaler Titel nach erfolgreichem Download
    pub track_id: Option<i64>,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

/// `POST /api/v1/downloads` → `Vec<DownloadJob>` (schon geladene oder laufende Titel
/// bekommen ihren bestehenden Auftrag zurück, nichts doppelt).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct DownloadRequest {
    pub tracks: Vec<YtTrack>,
}

/// `GET /api/v1/downloads/map` – welche YouTube-Titel als lokale Datei vorliegen.
/// Der Player spielt diese lokal statt über YouTube.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DownloadedTrack {
    pub video_id: String,
    pub track_id: i64,
}

/// `GET /api/v1/downloads/tool`, `POST /api/v1/downloads/tool/update`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct YtDlpInfo {
    /// yt-dlp gefunden und lauffähig
    pub available: bool,
    pub version: Option<String>,
    /// ffmpeg gefunden (für MP3 und das Umverpacken nötig)
    pub ffmpeg: bool,
    /// JavaScript-Laufzeit für YouTube (z. B. "deno 2.9.8"), `None` = keine gefunden
    pub js_runtime: Option<String>,
    /// Cookie-Datei vorhanden (`data/youtube-cookies.txt`)
    pub cookies: bool,
    pub last_update: Option<i64>,
    /// Letzte Meldung von `yt-dlp -U` oder letzter Fehler
    pub message: Option<String>,
}

// ---------------------------------------------------------------- Spotify

/// `GET /api/v1/spotify/status`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SpotifyStatus {
    /// Client-ID eingetragen
    pub configured: bool,
    /// Angemeldet (gültiges Refresh-Token)
    pub connected: bool,
    pub user: Option<SpotifyUser>,
    pub client_id: String,
    /// Diese Redirect-URI muss in der Spotify-App eingetragen sein
    pub redirect_uri: String,
    /// Verbunden über das Webplayer-Cookie `sp_dc` (ohne eigene Spotify-App)
    #[serde(default)]
    pub via_cookie: bool,
}

/// `POST /api/v1/spotify/cookie` – Wert des Cookies `sp_dc` von open.spotify.com (auch
/// `sp_dc=…` oder eine ganze Cookie-Zeile).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SpotifyCookie {
    pub cookie: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpotifyUser {
    pub id: String,
    pub display_name: Option<String>,
    pub image: Option<String>,
    /// "premium", "free", …
    pub product: Option<String>,
}

/// Antwort auf `POST /api/v1/spotify/login`: diese Adresse im Browser öffnen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpotifyLogin {
    pub authorize_url: String,
}

/// `POST /api/v1/spotify/callback` – die Adresse, auf die Spotify nach der Anmeldung
/// weitergeleitet hat (mit `?code=…&state=…`), aus der Adresszeile eingefügt. Nötig, wenn
/// die Redirect-URI nicht direkt zu nrzMusic führt (z. B. `http://127.0.0.1:…` auf Umbrel).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SpotifyCallback {
    pub url: String,
}

/// Ein Titel aus Spotify. Abgespielt wird er über den passenden YouTube-Music-Titel
/// (oder dessen Download).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpTrack {
    /// Spotify-Track-ID
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration_ms: u64,
    /// Album-Cover (möglichst ≥ 300 px)
    pub image: Option<String>,
    pub isrc: Option<String>,
    #[serde(default)]
    pub explicit: bool,
    /// Wann zu den Lieblingssongs/zur Playlist hinzugefügt (ISO 8601)
    pub added_at: Option<String>,
    /// Gefundener YouTube-Music-Titel, falls schon abgeglichen (vom Server zwischengespeichert)
    pub youtube: Option<YtTrack>,
}

impl SpTrack {
    /// "A, B & C"
    pub fn artist_line(&self) -> String {
        match self.artists.len() {
            0 => String::new(),
            1 => self.artists[0].clone(),
            n => format!("{} & {}", self.artists[..n - 1].join(", "), self.artists[n - 1]),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpPlaylistSummary {
    pub id: String,
    pub name: String,
    pub owner: Option<String>,
    pub track_count: u32,
    pub image: Option<String>,
    /// Titel abrufbar: Spotify-Apps im Entwicklungsmodus sehen nur Titel eigener und
    /// kollaborativer Playlists, gefolgte Playlists anderer liefern nur diese Übersicht.
    #[serde(default = "readable_default")]
    pub readable: bool,
}

fn readable_default() -> bool {
    true
}

/// `GET /api/v1/spotify/playlists/<id>`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpPlaylist {
    pub summary: SpPlaylistSummary,
    pub tracks: Vec<SpTrack>,
}

/// `POST /api/v1/spotify/match` → `Vec<SpTrack>` in derselben Reihenfolge, `youtube` gefüllt,
/// wo ein Treffer gefunden wurde.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SpMatchRequest {
    pub tracks: Vec<SpTrack>,
}

/// `POST /api/v1/spotify/import` → `202`. Läuft im Hintergrund (`SpotifyProgress`):
/// gleicht ab, legt eine nrzMusic-Playlist mit den YouTube-Treffern an und lädt sie auf
/// Wunsch gleich herunter.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SpImportRequest {
    /// `"liked"` für die Lieblingssongs oder eine Spotify-Playlist-ID
    pub source: String,
    /// Playlist in nrzMusic anlegen
    #[serde(default)]
    pub create_playlist: bool,
    /// Alle Treffer mit yt-dlp in den Musikordner laden
    #[serde(default)]
    pub download: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yt() -> YtTrack {
        YtTrack {
            video_id: "abc".into(),
            title: "T".into(),
            artists: vec![
                YtArtistRef { name: "A".into(), id: None },
                YtArtistRef { name: "B".into(), id: None },
                YtArtistRef { name: "C".into(), id: None },
            ],
            album: None,
            duration_ms: Some(1000),
            thumbnail: None,
            explicit: false,
            is_video: false,
        }
    }

    #[test]
    fn artist_line() {
        assert_eq!(yt().artist_line(), "A, B & C");
    }

    #[test]
    fn playable_json_is_tagged() {
        let p = PlayableTrack::Youtube(yt());
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"source\":\"youtube\""));
        assert_eq!(serde_json::from_str::<PlayableTrack>(&json).unwrap(), p);
        assert_eq!(p.key(), "yt:abc");
        let r = serde_json::to_string(&TrackRef::Local { id: 7 }).unwrap();
        assert_eq!(r, "{\"source\":\"local\",\"id\":7}");
    }

    #[test]
    fn old_settings_json_still_parses() {
        let s: Settings = serde_json::from_str(
            r#"{"youtube_enabled":false,"youtube_language":"en","youtube_region":"US"}"#,
        )
        .unwrap();
        assert!(!s.youtube_enabled);
        assert_eq!(s.download_format, "mp3");
        assert!(s.spotify_client_id.is_empty());
    }

    #[test]
    fn download_event_is_tagged() {
        let job = DownloadJob {
            id: 1,
            track: yt(),
            status: DownloadStatus::Running,
            progress: 0.5,
            track_id: None,
            error: None,
            created_at: 0,
            finished_at: None,
        };
        let json = serde_json::to_string(&ServerEvent::DownloadUpdate(Box::new(job.clone()))).unwrap();
        assert!(json.contains("\"type\":\"download_update\""));
        assert!(json.contains("\"status\":\"running\""));
        assert_eq!(
            serde_json::from_str::<ServerEvent>(&json).unwrap(),
            ServerEvent::DownloadUpdate(Box::new(job))
        );
    }

    #[test]
    fn yt_item_keeps_album_kind() {
        let item = YtItem::Album(YtAlbumSummary {
            browse_id: "MPREb_x".into(),
            title: "A".into(),
            artists: vec![],
            year: None,
            kind: "Single".into(),
            thumbnail: None,
        });
        let json = serde_json::to_string(&item).unwrap();
        assert_eq!(serde_json::from_str::<YtItem>(&json).unwrap(), item);
    }
}

// ---------------------------------------------------------------- Konten

/// Ein Konto von nrzMusic (ohne Passwort).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Account {
    pub id: i64,
    pub username: String,
    /// Angezeigter Name (leer = Benutzername)
    pub display_name: String,
    pub is_admin: bool,
    pub created_at: i64,
    pub last_login: Option<i64>,
}

impl Account {
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.username
        } else {
            self.display_name.trim()
        }
    }
}

/// `GET /api/v1/auth/status`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AuthStatus {
    /// Konten eingeschaltet (`NRZMUSIC_AUTH`); aus = alle sind der eine lokale Nutzer
    pub enabled: bool,
    /// Angemeldet als
    pub user: Option<Account>,
    /// Neue Konten erlaubt
    pub registration_open: bool,
    /// Noch kein Konto da – das erste wird Admin
    pub setup: bool,
}

/// `POST /api/v1/auth/login`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// `POST /api/v1/auth/register`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
}

/// `PUT /api/v1/auth/password`
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PasswordChange {
    pub current: String,
    pub new: String,
}

/// `PATCH /api/v1/auth/account` (eigenes Konto)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AccountUpdate {
    pub display_name: Option<String>,
}

/// `PATCH /api/v1/users/<id>` (nur Admin)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct UserUpdate {
    pub is_admin: Option<bool>,
    /// Neues Passwort setzen
    pub password: Option<String>,
}

// ---------------------------------------------------------------- Feed, Hörverhalten, Statistik

/// `POST /api/v1/listens` – eine Wiedergabe ist vorbei (Titel gewechselt, zu Ende, gestoppt).
/// Daraus lernt der Feed (gehörte Zeit, Überspringen) und entsteht die Statistik.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ListenReport {
    pub track: TrackRef,
    /// Tatsächlich gehörte Zeit (ohne Pausen und Spulen)
    pub listened_ms: u64,
    /// Länge des Titels (0 = unbekannt)
    #[serde(default)]
    pub duration_ms: u64,
    /// Vor dem Ende weitergeschaltet
    #[serde(default)]
    pub skipped: bool,
    /// Bis zum Ende gelaufen
    #[serde(default)]
    pub completed: bool,
    /// Woher der Titel kam: "feed" oder leer
    #[serde(default)]
    pub source: String,
}

/// `POST /api/v1/feed` – nächste Seite des unendlichen Feeds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct FeedRequest {
    /// Schon gezeigte Schlüssel (kommen nicht noch einmal)
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Gewünschte Anzahl (Standard 20, höchstens 50)
    #[serde(default)]
    pub count: Option<u32>,
}

/// Warum ein Titel im Feed steht.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedReason {
    /// Ähnlich wie ein Titel, den du magst
    Similar { to: String },
    /// Mehr von einem Künstler, den du oft hörst
    Artist { name: String },
    /// Lange nicht gehört, aber gemocht
    Rediscover,
    /// Aus deiner Bibliothek
    Library,
    /// Etwas Neues ausprobieren
    Explore,
    /// Gerade beliebt auf YouTube Music
    Trending,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FeedItem {
    pub track: PlayableTrack,
    pub reason: FeedReason,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct FeedPage {
    pub items: Vec<FeedItem>,
    /// Noch kaum Hörverhalten bekannt – der Feed rät mehr
    #[serde(default)]
    pub cold_start: bool,
}

/// `POST /api/v1/feed/dislike` – "Nicht interessiert".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DislikeRequest {
    pub track: TrackRef,
}

/// `GET`/`PUT /api/v1/prefs` – Einstellungen des eigenen Kontos.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct UserPrefs {
    /// YouTube-Titel beim Liken gleich herunterladen
    #[serde(default)]
    pub auto_download_likes: bool,
}

/// Ein Eintrag einer Rangliste (Künstler, Genre).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatEntry {
    pub name: String,
    pub ms: u64,
    pub plays: u64,
    /// Bild (Cover eines gehörten Titels)
    pub image: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatTrack {
    pub track: PlayableTrack,
    pub ms: u64,
    pub plays: u64,
}

/// Ein Balken im Zeitverlauf: Tag (`start` = lokaler Tagesbeginn) oder Monat.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatBucket {
    /// Unix-Sekunden des Beginns (lokale Zeit als UTC gerechnet – nur zum Beschriften)
    pub start: i64,
    pub ms: u64,
}

/// `GET /api/v1/stats?days=<n>&tz=<minuten>` – `days` 0 = alles; `tz` wie
/// `Date.getTimezoneOffset()`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Stats {
    pub days: u32,
    pub total_ms: u64,
    pub plays: u64,
    pub skips: u64,
    pub tracks: u64,
    pub artists: u64,
    pub top_artists: Vec<StatEntry>,
    pub top_tracks: Vec<StatTrack>,
    pub top_genres: Vec<StatEntry>,
    /// 24 Werte: gehörte Zeit je Stunde (lokal)
    pub by_hour: Vec<u64>,
    /// 7 Werte: Montag … Sonntag
    pub by_weekday: Vec<u64>,
    /// "day" oder "month"
    pub bucket: String,
    pub timeline: Vec<StatBucket>,
    /// Erste aufgezeichnete Wiedergabe
    pub since: Option<i64>,
}

// ---------------------------------------------------------------- Teilen

/// Was geteilt wird.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareKind {
    /// Ein einzelner Titel
    Track,
    /// Die eigenen Likes (Favoriten) – immer der aktuelle Stand
    Favorites,
    /// Eine eigene Playlist – immer der aktuelle Stand
    Playlist,
}

/// `POST /api/v1/shares` – Link anlegen (gibt es ihn schon, kommt derselbe zurück).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShareRequest {
    pub kind: ShareKind,
    #[serde(default)]
    pub track: Option<TrackRef>,
    #[serde(default)]
    pub playlist_id: Option<i64>,
}

/// Ein eigener Link (`GET /api/v1/shares`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShareInfo {
    pub token: String,
    pub kind: ShareKind,
    /// Titel bzw. Playlist-Name ("" bei Likes)
    pub title: String,
    /// Pfad des Links: `/s/<token>`
    pub path: String,
    pub created_at: i64,
    pub views: u64,
}

/// `GET /api/v1/public/share/<token>` – ohne Anmeldung lesbar.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SharedView {
    pub token: String,
    pub kind: ShareKind,
    pub title: String,
    pub description: String,
    /// Wer geteilt hat (Anzeigename)
    pub owner: String,
    pub artwork: Option<String>,
    pub tracks: Vec<PlayableTrack>,
}
