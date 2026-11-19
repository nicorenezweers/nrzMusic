//! Einstellungen der App (`~/Library/Application Support/nrzMusic/app.json`): lokal oder mit
//! welcher Server-Instanz. Beim ersten Start steht noch nichts darin – dann fragt die App.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Noch nicht gewählt (erster Start).
    #[default]
    Unset,
    /// Eingebauter Server auf diesem Mac.
    Local,
    /// Verbunden mit `server`.
    Server,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    /// Zuletzt verbundene Server-Instanz, z. B. `https://music.zweers.ch`.
    pub server: String,
    /// Lage und Grösse des Fensters beim letzten Mal (logische Punkte).
    pub window: Option<Frame>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// `~/Library/Application Support/nrzMusic`
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NRZMUSIC_APP_DIR") {
        return PathBuf::from(dir);
    }
    home().join("Library/Application Support/nrzMusic")
}

/// `~/Library/Logs/nrzMusic`
pub fn log_dir() -> PathBuf {
    home().join("Library/Logs/nrzMusic")
}

/// `~/Music/nrzMusic` – Musikordner des lokalen Modus.
pub fn music_dir() -> PathBuf {
    home().join("Music/nrzMusic")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

fn path() -> PathBuf {
    data_dir().join("app.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(data_dir());
        let tmp = p.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}

/// Eingabe zu einer Adresse: ohne Schema zuerst `https://`, bei IP-Adressen, `localhost` und
/// `.local` `http://`; ohne Schrägstrich am Ende.
pub fn candidates(input: &str) -> Vec<String> {
    let s = input.trim().trim_end_matches('/');
    if s.is_empty() {
        return Vec::new();
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return vec![s.to_string()];
    }
    let host = s.split(['/', ':']).next().unwrap_or("");
    let plain = host == "localhost"
        || host.ends_with(".local")
        || host.parse::<std::net::Ipv4Addr>().is_ok();
    if plain {
        vec![format!("http://{s}"), format!("https://{s}")]
    } else {
        vec![format!("https://{s}"), format!("http://{s}")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(candidates(" https://music.zweers.ch/ "), vec!["https://music.zweers.ch"]);
        assert_eq!(candidates("music.zweers.ch")[0], "https://music.zweers.ch");
        assert_eq!(candidates("192.168.1.5:8350")[0], "http://192.168.1.5:8350");
        assert_eq!(candidates("umbrel.local:8350")[0], "http://umbrel.local:8350");
        assert!(candidates("  ").is_empty());
    }

    #[test]
    fn mode_as_json() {
        let s: Settings = serde_json::from_str(r#"{"mode":"server","server":"https://x"}"#).unwrap();
        assert_eq!(s.mode, Mode::Server);
        assert_eq!(Settings::default().mode, Mode::Unset);
    }
}
