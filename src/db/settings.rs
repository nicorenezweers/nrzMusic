//! Einstellungen der Oberfläche (`Settings`), als Schlüssel/Wert in der Tabelle `settings`.
//! [`SettingsStore`] hält sie zusätzlich im Speicher (Rocket-State, billig zu klonen), damit
//! nicht jede YouTube-Anfrage oder jeder Download-Auftrag die Datenbank fragen muss.
//!
//! Fehlende Schlüssel (ältere Datenbank) nehmen den Standardwert aus `Settings::default()`.

use crate::error::AppResult;
use nrzmusic_types::Settings;
use sqlx::SqlitePool;
use std::sync::{Arc, RwLock};

/// Erlaubte Zielformate der Downloads.
pub const DOWNLOAD_FORMATS: &[&str] = &["mp3", "m4a", "opus"];

pub async fn load(pool: &SqlitePool) -> AppResult<Settings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    let mut s = Settings::default();
    for (key, value) in rows {
        match key.as_str() {
            "youtube_enabled" => s.youtube_enabled = value == "true",
            "youtube_language" if !value.is_empty() => s.youtube_language = value,
            "youtube_region" if !value.is_empty() => s.youtube_region = value,
            "downloads_enabled" => s.downloads_enabled = value == "true",
            "download_format" if DOWNLOAD_FORMATS.contains(&value.as_str()) => {
                s.download_format = value
            }
            "ytdlp_auto_update" => s.ytdlp_auto_update = value == "true",
            "spotify_client_id" => s.spotify_client_id = value,
            "spotify_redirect_uri" => s.spotify_redirect_uri = value,
            "registration_open" => s.registration_open = value == "true",
            _ => {}
        }
    }
    Ok(s)
}

pub async fn save(pool: &SqlitePool, s: &Settings) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    for (key, value) in [
        ("youtube_enabled", s.youtube_enabled.to_string()),
        ("youtube_language", s.youtube_language.clone()),
        ("youtube_region", s.youtube_region.clone()),
        ("downloads_enabled", s.downloads_enabled.to_string()),
        ("download_format", s.download_format.clone()),
        ("ytdlp_auto_update", s.ytdlp_auto_update.to_string()),
        ("spotify_client_id", s.spotify_client_id.clone()),
        ("spotify_redirect_uri", s.spotify_redirect_uri.clone()),
        ("registration_open", s.registration_open.to_string()),
    ] {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Einstellungen im Speicher plus die Vorgaben aus der Umgebung (`NRZMUSIC_YOUTUBE`,
/// `NRZMUSIC_DOWNLOADS`, `NRZMUSIC_SPOTIFY_CLIENT_ID`).
#[derive(Clone)]
pub struct SettingsStore {
    current: Arc<RwLock<Settings>>,
    youtube_allowed: bool,
    downloads_allowed: bool,
    /// Client-ID aus der Umgebung – gilt, solange in den Einstellungen keine steht
    spotify_client_id: String,
}

impl SettingsStore {
    pub fn new(settings: Settings, youtube_allowed: bool) -> Self {
        SettingsStore {
            current: Arc::new(RwLock::new(settings)),
            youtube_allowed,
            downloads_allowed: true,
            spotify_client_id: String::new(),
        }
    }

    /// Schalter für Downloads und Spotify-Voreinstellung aus der Umgebung.
    pub fn with_env(mut self, downloads_allowed: bool, spotify_client_id: &str) -> Self {
        self.downloads_allowed = downloads_allowed;
        self.spotify_client_id = spotify_client_id.trim().to_string();
        self
    }

    /// Wie gespeichert – `youtube_enabled`/`downloads_enabled` sind aus, wenn die Umgebung sie
    /// abschaltet; eine leere Spotify-Client-ID nimmt die Voreinstellung aus der Umgebung.
    pub fn get(&self) -> Settings {
        let mut s = self.current.read().unwrap().clone();
        s.youtube_enabled &= self.youtube_allowed;
        s.downloads_enabled &= self.downloads_allowed;
        if s.spotify_client_id.trim().is_empty() {
            s.spotify_client_id = self.spotify_client_id.clone();
        }
        s
    }

    pub fn set(&self, settings: Settings) {
        *self.current.write().unwrap() = settings;
    }

    /// Ist YouTube Music gerade an (Umgebung und Einstellung)?
    pub fn youtube_enabled(&self) -> bool {
        self.youtube_allowed && self.current.read().unwrap().youtube_enabled
    }

    /// Hat die Umgebung YouTube Music erlaubt?
    pub fn youtube_allowed(&self) -> bool {
        self.youtube_allowed
    }

    /// Sind Downloads gerade an (Umgebung und Einstellung)?
    pub fn downloads_enabled(&self) -> bool {
        self.downloads_allowed && self.current.read().unwrap().downloads_enabled
    }

    /// Kommt die angezeigte Spotify-Client-ID aus der Umgebung (gespeichert ist keine)?
    pub fn spotify_client_id_is_preset(&self) -> bool {
        !self.spotify_client_id.is_empty()
            && self
                .current
                .read()
                .unwrap()
                .spotify_client_id
                .trim()
                .is_empty()
    }

    /// Hat die Umgebung Downloads erlaubt?
    pub fn downloads_allowed(&self) -> bool {
        self.downloads_allowed
    }
}
