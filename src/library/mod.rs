//! Der Musikordner: Scanner (`scanner`), Überwachung (`watcher`), Tags (`tags`, mit lofty)
//! und Cover (`covers`). Die Dateien bleiben, wo sie sind – die Datenbank kennt nur ihre
//! Metadaten und Pfade relativ zum Musikordner.
//!
//! [`Library`] ist der gemeinsame Zustand (Rocket-State): Pfade, Datenbank, Scan-Status und
//! der Kanal für Server-Sent Events.

pub mod covers;
pub mod scanner;
pub mod tags;
pub mod watcher;

use crate::config::Config;
use nrzmusic_types::{ScanProgress, ServerEvent};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Dateiendungen, die als Musik gelten (klein geschrieben).
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "aif", "aiff",
];

/// Endungen halb fertiger Downloads/Kopien – solche Dateien werden nie gelesen.
const PARTIAL_SUFFIXES: &[&str] = &[".part", ".crdownload", ".tmp", ".download"];

/// Dateiendung in Kleinbuchstaben, wenn sie zu einem Audioformat gehört.
pub fn audio_format(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    AUDIO_EXTENSIONS.contains(&ext.as_str()).then_some(ext)
}

/// Versteckte Dateien/Ordner (".…") und halb fertige Downloads überspringen.
pub fn is_ignored_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.') || PARTIAL_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// Kurzer Hash (die ersten 16 Hex-Zeichen von SHA-256).
pub fn short_hash(data: impl AsRef<[u8]>) -> String {
    let mut hex = hex::encode(Sha256::digest(data.as_ref()));
    hex.truncate(16);
    hex
}

/// Stabile Album-ID aus Album-Künstler und Albumtitel (Gross/Klein egal).
pub fn album_id(album_artist: &str, album: &str) -> String {
    short_hash(format!(
        "{}\u{1f}{}",
        album_artist.trim().to_lowercase(),
        album.trim().to_lowercase()
    ))
}

/// Stabile Künstler-ID aus dem Namen (Gross/Klein egal).
pub fn artist_id(name: &str) -> String {
    short_hash(name.trim().to_lowercase())
}

/// Für die Suche: klein geschrieben, gängige Akzente entfernt ("Beyoncé" → "beyonce").
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        let mapped: &str = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'æ' => "ae",
            'ç' | 'ć' | 'č' => "c",
            'ď' | 'đ' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
            'ğ' => "g",
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => "i",
            'ł' => "l",
            'ñ' | 'ń' | 'ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
            'œ' => "oe",
            'ř' => "r",
            'ß' => "ss",
            'ś' | 'š' | 'ş' => "s",
            'ť' | 'ţ' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => "u",
            'ý' | 'ÿ' => "y",
            'ź' | 'ż' | 'ž' => "z",
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push_str(mapped);
    }
    out
}

/// Ereignisse für `GET /api/v1/events` (Server-Sent Events). `user` = nur für dieses Konto
/// (Playlists, Favoriten, Spotify), `None` = für alle (Bibliothek, Downloads).
#[derive(Clone)]
pub struct Events(broadcast::Sender<(Option<i64>, ServerEvent)>);

impl Default for Events {
    fn default() -> Self {
        Self::new()
    }
}

impl Events {
    pub fn new() -> Self {
        Events(broadcast::channel(256).0)
    }

    /// An alle senden; ohne Zuhörer geht das Ereignis einfach verloren.
    pub fn send(&self, event: ServerEvent) {
        let _ = self.0.send((None, event));
    }

    /// Nur an die Verbindungen eines Kontos.
    pub fn send_to(&self, user: i64, event: ServerEvent) {
        let _ = self.0.send((Some(user), event));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(Option<i64>, ServerEvent)> {
        self.0.subscribe()
    }
}

/// Gemeinsamer Zustand der Bibliothek (billig zu klonen).
#[derive(Clone)]
pub struct Library {
    inner: Arc<Inner>,
}

struct Inner {
    /// Musikordner, kanonisch (Ereignisse der Überwachung tragen kanonische Pfade)
    root: PathBuf,
    /// Musikordner, wie konfiguriert (für die Anzeige)
    display_root: String,
    covers: PathBuf,
    pool: SqlitePool,
    events: Events,
    scanning: AtomicBool,
    progress: Mutex<Option<ScanProgress>>,
    /// Ende des letzten vollen Scans (Unix-Sekunden), 0 = noch keiner
    last_scan: AtomicI64,
    watching: AtomicBool,
}

impl Library {
    pub fn new(pool: SqlitePool, config: &Config, events: Events) -> Self {
        let root = std::fs::canonicalize(&config.music_dir).unwrap_or_else(|_| {
            std::path::absolute(&config.music_dir).unwrap_or_else(|_| config.music_dir.clone())
        });
        Library {
            inner: Arc::new(Inner {
                root,
                display_root: config.music_dir.display().to_string(),
                covers: config.covers_dir(),
                pool,
                events,
                scanning: AtomicBool::new(false),
                progress: Mutex::new(None),
                last_scan: AtomicI64::new(0),
                watching: AtomicBool::new(false),
            }),
        }
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn display_root(&self) -> &str {
        &self.inner.display_root
    }

    pub fn covers_dir(&self) -> &Path {
        &self.inner.covers
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    pub fn events(&self) -> &Events {
        &self.inner.events
    }

    pub fn is_scanning(&self) -> bool {
        self.inner.scanning.load(Ordering::SeqCst)
    }

    pub fn progress(&self) -> Option<ScanProgress> {
        *self.inner.progress.lock().unwrap()
    }

    fn set_progress(&self, progress: Option<ScanProgress>) {
        *self.inner.progress.lock().unwrap() = progress;
    }

    pub fn last_scan(&self) -> Option<i64> {
        Some(self.inner.last_scan.load(Ordering::SeqCst)).filter(|t| *t > 0)
    }

    pub fn is_watching(&self) -> bool {
        self.inner.watching.load(Ordering::SeqCst)
    }

    pub fn set_watching(&self, watching: bool) {
        self.inner.watching.store(watching, Ordering::SeqCst);
    }

    /// Absoluter Pfad zu einem Pfad relativ zum Musikordner.
    pub fn abs_path(&self, rel: &str) -> PathBuf {
        self.inner.root.join(rel)
    }

    /// Pfad relativ zum Musikordner ('/'-getrennt), `None` ausserhalb oder für den Ordner selbst.
    pub fn rel_path(&self, abs: &Path) -> Option<String> {
        let rel = abs.strip_prefix(&self.inner.root).ok()?;
        let mut parts = Vec::new();
        for c in rel.components() {
            match c {
                Component::Normal(p) => parts.push(p.to_str()?.to_string()),
                _ => return None,
            }
        }
        (!parts.is_empty()).then(|| parts.join("/"))
    }
}

/// Liegt der relative Pfad in einem versteckten Ordner oder ist selbst ausgeblendet?
pub fn is_ignored_rel(rel: &str) -> bool {
    rel.split('/').any(is_ignored_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_ignore_case_and_whitespace() {
        assert_eq!(album_id("ABBA", "Gold "), album_id("abba", "gold"));
        assert_ne!(album_id("ABBA", "Gold"), album_id("ABBA", "Arrival"));
        assert_eq!(artist_id("Queen").len(), 16);
    }

    #[test]
    fn folding() {
        assert_eq!(
            fold("Beyoncé – Ärzte STRASSE ß"),
            "beyonce – arzte strasse ss"
        );
    }

    #[test]
    fn ignored_names() {
        assert!(is_ignored_name(".hidden.mp3"));
        assert!(is_ignored_name("song.mp3.part"));
        assert!(is_ignored_name("song.MP3.crdownload"));
        assert!(!is_ignored_name("song.mp3"));
        assert!(is_ignored_rel("a/.b/c.mp3"));
        assert_eq!(audio_format(Path::new("x/Y.FLAC")).as_deref(), Some("flac"));
        assert_eq!(audio_format(Path::new("x/y.txt")), None);
    }
}
