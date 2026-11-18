//! Upload in den Musikordner (`POST /api/v1/upload`, multipart, Feld `files` mehrfach).
//!
//! Der Dateiname darf einen relativen Pfad tragen ("Album/01 Song.mp3", z. B. beim Hochladen
//! eines Ordners). Jeder Teil wird bereinigt; `..`, absolute Pfade und versteckte Namen werden
//! abgewiesen, nur Audio-Endungen angenommen. Bestehende Dateien werden nie überschrieben –
//! dann heisst die neue "Song (2).mp3". Die Datei wird zuerst versteckt neben das Ziel
//! geschrieben (die Überwachung ignoriert sie) und dann unter dem freien Namen verlinkt.

use crate::db::library as store;
use crate::error::AppResult;
use crate::library::{audio_format, Library};
use nrzmusic_types::{UploadResult, UploadSkipped};
use rocket::form::Form;
use rocket::fs::TempFile;
use rocket::serde::json::Json;
use rocket::{post, routes, FromForm, Route, State};
use std::path::{Path, PathBuf};

pub fn routes() -> Vec<Route> {
    routes![upload]
}

#[derive(FromForm)]
pub struct UploadForm<'r> {
    files: Vec<TempFile<'r>>,
}

/// Zeichen, die in Dateinamen (spätestens unter Windows/SMB) Ärger machen.
pub(crate) fn clean_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    cleaned.trim().trim_end_matches(['.', ' ']).to_string()
}

/// Höchstens `max` Bytes, an einer Zeichengrenze gekürzt.
pub(crate) fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].trim_end().to_string()
}

/// Rohen Dateinamen (evtl. mit relativem Pfad) in sichere Pfadteile zerlegen.
/// Fehler: Begründung für `UploadSkipped.reason`.
pub fn sanitize_upload_path(raw: &str) -> Result<Vec<String>, &'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Dateiname fehlt.");
    }
    let bytes = raw.as_bytes();
    if raw.starts_with('/')
        || raw.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
    {
        return Err("Absolute Pfade sind nicht erlaubt.");
    }
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        let part = part.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err("Pfade mit „..“ sind nicht erlaubt.");
        }
        if part.starts_with('.') {
            return Err("Versteckte Dateien und Ordner werden nicht übernommen.");
        }
        let cleaned = clean_component(part);
        if cleaned.is_empty() || cleaned.starts_with('.') {
            return Err("Ungültiger Dateiname.");
        }
        parts.push(cleaned);
    }
    if parts.is_empty() {
        return Err("Ungültiger Dateiname.");
    }
    if parts.len() > 16 {
        return Err("Zu viele Ordnerebenen.");
    }
    let last = parts.len() - 1;
    for p in &mut parts[..last] {
        *p = truncate_bytes(p, 200);
    }
    let name = parts[last].clone();
    let Some(ext) = audio_format(Path::new(&name)) else {
        return Err("Kein unterstütztes Audioformat.");
    };
    // Lange Namen kürzen, die Endung bleibt
    let stem = &name[..name.len() - ext.len() - 1];
    parts[last] = format!(
        "{}.{}",
        truncate_bytes(stem, 200),
        &name[name.len() - ext.len()..]
    );
    Ok(parts)
}

/// "Song.mp3" → "Song (2).mp3"
pub(crate) fn numbered(name: &str, n: u32) -> String {
    if n <= 1 {
        return name.to_string();
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem} ({n}).{ext}"),
        None => format!("{name} ({n})"),
    }
}

/// Zielordner (Teile relativ zum Musikordner) anlegen. Ein Ordner-Symlink darf nicht aus dem
/// Musikordner hinausführen.
pub(crate) async fn prepare_dir(root: &Path, dirs: &[String]) -> std::io::Result<PathBuf> {
    let mut dir = root.to_path_buf();
    for d in dirs {
        dir.push(d);
    }
    tokio::fs::create_dir_all(&dir).await?;
    let canonical = tokio::fs::canonicalize(&dir).await?;
    let root_canonical = tokio::fs::canonicalize(root).await?;
    if !canonical.starts_with(&root_canonical) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Ziel liegt ausserhalb des Musikordners",
        ));
    }
    Ok(dir)
}

/// Versteckter Name für eine Datei, die gerade in `dir` geschrieben wird (die Überwachung
/// ignoriert ihn).
pub(crate) fn hidden_temp_name(dir: &Path, prefix: &str) -> PathBuf {
    dir.join(format!(
        ".nrzmusic-{prefix}-{}.part",
        hex::encode(rand::random::<[u8; 8]>())
    ))
}

/// Fertig geschriebene, versteckte Datei `tmp` (im Ordner `dir`) unter dem ersten freien Namen
/// sichtbar machen: "Song.mp3", "Song (2).mp3", … – nie überschreiben. Lesbar für alle.
pub(crate) async fn publish_temp(tmp: &Path, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let result = tokio::task::spawn_blocking({
        let (tmp, dir, name) = (tmp.to_path_buf(), dir.to_path_buf(), name.to_string());
        move || -> std::io::Result<PathBuf> {
            for n in 1..=999 {
                let target = dir.join(numbered(&name, n));
                // hard_link scheitert, wenn es das Ziel schon gibt – nie überschreiben
                match std::fs::hard_link(&tmp, &target) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&tmp);
                        return Ok(target);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(_) => {
                        // Dateisystem ohne Hardlinks: umbenennen, wenn der Name frei ist
                        if std::fs::symlink_metadata(&target).is_ok() {
                            continue;
                        }
                        std::fs::rename(&tmp, &target)?;
                        return Ok(target);
                    }
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "kein freier Dateiname",
            ))
        }
    })
    .await
    .map_err(std::io::Error::other)?;
    match &result {
        // Temporäre Dateien sind oft nur für den Besitzer lesbar – Musik soll es für alle sein
        #[cfg(unix)]
        Ok(target) => {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                tokio::fs::set_permissions(target, std::fs::Permissions::from_mode(0o644)).await;
        }
        #[cfg(not(unix))]
        Ok(_) => {}
        Err(_) => {
            let _ = tokio::fs::remove_file(tmp).await;
        }
    }
    result
}

/// Hochgeladene Datei an einen freien Namen im Musikordner legen; liefert den Pfad.
async fn place(root: &Path, parts: &[String], file: &mut TempFile<'_>) -> std::io::Result<PathBuf> {
    let (name, dirs) = parts.split_last().expect("mindestens ein Teil");
    let dir = prepare_dir(root, dirs).await?;
    let tmp = hidden_temp_name(&dir, "upload");
    // Gleiches Dateisystem: umbenennen; sonst kopieren
    if file.persist_to(&tmp).await.is_err() {
        if let Err(e) = file.move_copy_to(&tmp).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    }
    publish_temp(&tmp, &dir, name).await
}

#[post("/upload", data = "<form>")]
pub async fn upload(
    _admin: crate::auth::Admin,
    mut form: Form<UploadForm<'_>>,
    library: &State<Library>,
) -> AppResult<Json<UploadResult>> {
    let mut result = UploadResult::default();
    let mut ids = Vec::new();
    for file in form.files.iter_mut() {
        let raw = file
            .raw_name()
            .map(|n| n.dangerous_unsafe_unsanitized_raw().as_str().to_string())
            .unwrap_or_default();
        let skip = |reason: &str| UploadSkipped {
            name: raw.clone(),
            reason: reason.to_string(),
        };
        let parts = match sanitize_upload_path(&raw) {
            Ok(p) => p,
            Err(reason) => {
                result.skipped.push(skip(reason));
                continue;
            }
        };
        if file.len() == 0 {
            result.skipped.push(skip("Die Datei ist leer."));
            continue;
        }
        let path = match place(library.root(), &parts, file).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(file = %raw, "Upload nicht speicherbar: {e}");
                result
                    .skipped
                    .push(skip("Die Datei liess sich nicht speichern."));
                continue;
            }
        };
        match library.scan_file(&path, true).await {
            Ok(outcome) => match outcome.id() {
                Some(id) => {
                    tracing::info!(file = %path.display(), "Hochgeladen");
                    ids.push(id);
                }
                None => result
                    .skipped
                    .push(skip("Die Datei liess sich nicht lesen.")),
            },
            Err(e) => {
                tracing::warn!(file = %path.display(), "Hochgeladene Datei nicht lesbar: {e}");
                result
                    .skipped
                    .push(skip("Die Datei liess sich nicht lesen."));
            }
        }
    }
    if !ids.is_empty() {
        let tracks = store::get_many(library.pool(), &ids).await?;
        result.imported = ids
            .iter()
            .filter_map(|id| tracks.get(id).cloned())
            .collect();
        library.notify_changed().await;
    }
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing() {
        assert_eq!(
            sanitize_upload_path("Album/01 Song.mp3").unwrap(),
            vec!["Album", "01 Song.mp3"]
        );
        assert_eq!(
            sanitize_upload_path("a\\b\\c.FLAC").unwrap(),
            vec!["a", "b", "c.FLAC"]
        );
        assert_eq!(
            sanitize_upload_path("./x/./y?.ogg").unwrap(),
            vec!["x", "y_.ogg"]
        );
        assert!(sanitize_upload_path("../evil.mp3").is_err());
        assert!(sanitize_upload_path("a/../../evil.mp3").is_err());
        assert!(sanitize_upload_path("/etc/evil.mp3").is_err());
        assert!(sanitize_upload_path("C:\\evil.mp3").is_err());
        assert!(sanitize_upload_path(".hidden.mp3").is_err());
        assert!(sanitize_upload_path("a/.git/x.mp3").is_err());
        assert!(sanitize_upload_path("readme.txt").is_err());
        assert!(sanitize_upload_path("song.mp3.part").is_err());
        assert!(sanitize_upload_path("").is_err());
        let long = format!("{}.mp3", "ä".repeat(300));
        let p = sanitize_upload_path(&long).unwrap();
        assert!(p[0].len() <= 204 && p[0].ends_with(".mp3"));
        assert_eq!(numbered("Song.mp3", 2), "Song (2).mp3");
    }
}
