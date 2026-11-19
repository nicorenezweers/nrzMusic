//! Einstellungen. Quelle ist Rockets Figment: `Rocket.toml` und Umgebungsvariablen mit
//! dem Präfix `NRZMUSIC_` (z. B. `NRZMUSIC_MUSIC_DIR`, `NRZMUSIC_DATA_DIR`). Wo `Rocket.toml`
//! liegt, sagt `NRZMUSIC_CONFIG`.

use rocket::figment::providers::{Env, Format, Toml};
use rocket::figment::Figment;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Datenordner: Datenbank und Cover-Zwischenspeicher.
    pub data_dir: PathBuf,
    /// Der überwachte Musikordner; Uploads landen auch hier.
    pub music_dir: PathBuf,
    /// Minuten zwischen zwei vollen Scans (Sicherheitsnetz neben der Überwachung), 0 = aus.
    pub scan_interval: u64,
    /// `false` schaltet YouTube Music ganz ab (auch wenn es in den Einstellungen an ist).
    pub youtube: bool,
    /// Hintergrundarbeiten (erster Scan, Überwachung, periodischer Scan, Selbst-Aktualisierung
    /// von yt-dlp). Tests schalten sie aus.
    pub background: bool,
    /// HTTP-Port (Rockets `port`) – für die Standard-Redirect-URI von Spotify.
    pub port: u16,
    /// `false` schaltet Downloads mit yt-dlp ganz ab (auch wenn sie in den Einstellungen an sind).
    pub downloads: bool,
    /// yt-dlp-Programm (Name im PATH oder Pfad); wird nach `data/tools/yt-dlp` kopiert.
    pub ytdlp: String,
    /// ffmpeg-Programm (Name im PATH oder Pfad).
    pub ffmpeg: String,
    /// Gleichzeitige Downloads.
    pub download_workers: usize,
    /// Voreinstellung für die Spotify-Client-ID (gilt, solange in den Einstellungen keine steht).
    pub spotify_client_id: String,
    /// Konten mit Anmeldung. `false` = ein einziger lokaler Nutzer ohne Anmeldung (nur hinter
    /// einem Proxy mit eigener Anmeldung oder im eigenen Netz).
    pub auth: bool,
    /// Öffentliche Adresse, z. B. `https://music.zweers.ch` – für die Spotify-Redirect-URI
    /// und sichere Cookies. Leer = `http://127.0.0.1:<port>`.
    pub public_url: String,
    /// Genres von YouTube-Künstlern für die Statistik nachschlagen (iTunes-Suche von Apple,
    /// nur der Künstlername wird geschickt). `false` = nur Genres aus den Tags lokaler Dateien.
    pub genre_lookup: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            music_dir: PathBuf::from("./music"),
            scan_interval: 30,
            youtube: true,
            background: true,
            port: 8350,
            downloads: true,
            ytdlp: "yt-dlp".into(),
            ffmpeg: "ffmpeg".into(),
            download_workers: 2,
            spotify_client_id: String::new(),
            auth: true,
            public_url: String::new(),
            genre_lookup: true,
        }
    }
}

impl Config {
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("nrzmusic.db")
    }

    pub fn covers_dir(&self) -> PathBuf {
        self.data_dir.join("covers")
    }

    /// Ganz geladene Tonspuren von YouTube-Titeln für die Wiedergabe (Zwischenspeicher).
    pub fn stream_cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache").join("audio")
    }

    /// Eigene Kopie von yt-dlp (aktualisiert sich selbst).
    pub fn tools_dir(&self) -> PathBuf {
        self.data_dir.join("tools")
    }

    /// Arbeitsordner der laufenden Downloads (beim Start geleert).
    pub fn downloads_dir(&self) -> PathBuf {
        self.data_dir.join("downloads")
    }

    /// Cookies für YouTube (Netscape-Format), falls vorhanden – für yt-dlp `--cookies`.
    pub fn cookies_path(&self) -> PathBuf {
        self.data_dir.join("youtube-cookies.txt")
    }

    /// Redirect-URI für Spotify, wenn in den Einstellungen keine steht.
    pub fn default_spotify_redirect(&self) -> String {
        let base = self.public_url.trim().trim_end_matches('/');
        if base.is_empty() {
            format!("http://127.0.0.1:{}/api/v1/spotify/callback", self.port)
        } else {
            format!("{base}/api/v1/spotify/callback")
        }
    }

    /// Daten-, Cover- und Musikordner anlegen, falls sie fehlen.
    pub fn create_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.covers_dir())?;
        std::fs::create_dir_all(&self.music_dir)?;
        Ok(())
    }
}

/// Rocket-Konfiguration plus `NRZMUSIC_*`-Umgebung (statt `ROCKET_*`).
pub fn figment() -> Figment {
    Figment::from(rocket::Config::default())
        .merge(Toml::file(Env::var_or("NRZMUSIC_CONFIG", "Rocket.toml")).nested())
        // NRZMUSIC_LOG ist der tracing-Filter (main.rs), keine Rocket-Einstellung
        .merge(
            Env::prefixed("NRZMUSIC_")
                .ignore(&["PROFILE", "LOG", "CONFIG"])
                .global(),
        )
        .select(rocket::figment::Profile::from_env_or(
            "NRZMUSIC_PROFILE",
            rocket::Config::DEFAULT_PROFILE,
        ))
}
