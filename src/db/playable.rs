//! Titel-Schlüssel (`local:<id>`, `yt:<videoId>`) für Playlists, Favoriten und Verlauf.
//! YouTube-Titel bringen ihre Metadaten mit; sie liegen in `yt_tracks`, damit Einträge ohne
//! erneute Anfrage bei YouTube angezeigt werden können.

use crate::db::{library, now};
use crate::error::{AppError, AppResult};
use nrzmusic_types::{PlayableTrack, TrackRef, YtTrack};
use sqlx::SqlitePool;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Local(i64),
    Youtube(String),
}

impl Key {
    pub fn parse(s: &str) -> Option<Key> {
        if let Some(id) = s.strip_prefix("local:") {
            return id.parse().ok().map(Key::Local);
        }
        let id = s.strip_prefix("yt:")?;
        valid_video_id(id).then(|| Key::Youtube(id.to_string()))
    }
}

/// Video-IDs von YouTube: 11 Zeichen aus `[A-Za-z0-9_-]` – etwas grosszügiger geprüft.
pub fn valid_video_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Metadaten eines YouTube-Titels speichern bzw. aktualisieren.
pub async fn store_yt(pool: &SqlitePool, track: &YtTrack) -> AppResult<()> {
    if !valid_video_id(&track.video_id) {
        return Err(AppError::bad("Ungültige YouTube-Video-ID."));
    }
    sqlx::query(
        "INSERT INTO yt_tracks (video_id, data, updated_at) VALUES (?, ?, ?) \
         ON CONFLICT (video_id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
    )
    .bind(&track.video_id)
    .bind(serde_json::to_string(track).unwrap_or_default())
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Einen Verweis übernehmen: YouTube-Metadaten speichern, lokale Titel prüfen.
/// `None`, wenn der lokale Titel nicht (mehr) existiert.
pub async fn accept(pool: &SqlitePool, r: &TrackRef) -> AppResult<Option<String>> {
    match r {
        TrackRef::Local { id } => Ok(library::exists(pool, *id).await?.then(|| r.key())),
        TrackRef::Youtube(t) => {
            store_yt(pool, t).await?;
            Ok(Some(r.key()))
        }
    }
}

/// Schlüssel zu Titeln auflösen – gleiche Reihenfolge, `None` für Verwaistes
/// (lokaler Titel gelöscht, unbekannter Schlüssel).
pub async fn resolve(pool: &SqlitePool, keys: &[String]) -> AppResult<Vec<Option<PlayableTrack>>> {
    let parsed: Vec<Option<Key>> = keys.iter().map(|k| Key::parse(k)).collect();
    let local_ids: Vec<i64> = parsed
        .iter()
        .filter_map(|k| match k {
            Some(Key::Local(id)) => Some(*id),
            _ => None,
        })
        .collect();
    let video_ids: Vec<&str> = parsed
        .iter()
        .filter_map(|k| match k {
            Some(Key::Youtube(id)) => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let locals = library::get_many(pool, &local_ids).await?;
    let mut yts: HashMap<String, YtTrack> = HashMap::new();
    if !video_ids.is_empty() {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT video_id, data FROM yt_tracks WHERE video_id IN (SELECT value FROM json_each(?))",
        )
        .bind(serde_json::to_string(&video_ids).unwrap_or_default())
        .fetch_all(pool)
        .await?;
        for (id, data) in rows {
            match serde_json::from_str::<YtTrack>(&data) {
                Ok(t) => {
                    yts.insert(id, t);
                }
                Err(e) => tracing::warn!(video = %id, "Gespeicherter YouTube-Titel unlesbar: {e}"),
            }
        }
    }
    Ok(parsed
        .into_iter()
        .map(|k| match k? {
            Key::Local(id) => locals.get(&id).cloned().map(PlayableTrack::Local),
            Key::Youtube(id) => yts.get(&id).cloned().map(PlayableTrack::Youtube),
        })
        .collect())
}

/// Nur die auflösbaren Einträge, mit ihrem Schlüssel.
pub async fn resolve_present(
    pool: &SqlitePool,
    keys: Vec<String>,
) -> AppResult<Vec<(String, PlayableTrack)>> {
    let resolved = resolve(pool, &keys).await?;
    Ok(keys
        .into_iter()
        .zip(resolved)
        .filter_map(|(k, t)| t.map(|t| (k, t)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(Key::parse("local:12"), Some(Key::Local(12)));
        assert_eq!(
            Key::parse("yt:dQw4w9WgXcQ"),
            Some(Key::Youtube("dQw4w9WgXcQ".into()))
        );
        assert_eq!(Key::parse("local:x"), None);
        assert_eq!(Key::parse("yt:../x"), None);
        assert_eq!(Key::parse("spotify:1"), None);
    }
}
