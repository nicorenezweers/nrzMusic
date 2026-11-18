//! Favoriten (Herz) und Verlauf – beide über Titel-Schlüssel, lokal und YouTube gemischt.

use crate::db::playable::{self, Key};
use crate::db::{library, now};
use crate::error::{AppError, AppResult};
use nrzmusic_types::{PlayableTrack, TrackRef};
use sqlx::SqlitePool;

/// Favorit setzen (bleibt bei erneutem Setzen an seiner Stelle).
pub async fn add(pool: &SqlitePool, uid: i64, r: &TrackRef) -> AppResult<()> {
    let key = playable::accept(pool, r).await?.ok_or(AppError::NotFound)?;
    sqlx::query(
        "INSERT INTO favorites (user_id, key, added_at) VALUES (?, ?, ?) \
         ON CONFLICT (user_id, key) DO NOTHING",
    )
        .bind(uid)
        .bind(key)
        .bind(now())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn remove(pool: &SqlitePool, uid: i64, key: &str) -> AppResult<bool> {
    let res = sqlx::query("DELETE FROM favorites WHERE user_id = ? AND key = ?")
        .bind(uid)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Alle Schlüssel (ohne verwaiste lokale Titel).
pub async fn keys(pool: &SqlitePool, uid: i64) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT f.key FROM favorites f \
         WHERE f.user_id = ? AND (f.key NOT LIKE 'local:%' \
            OR EXISTS (SELECT 1 FROM tracks t WHERE t.id = CAST(substr(f.key, 7) AS INTEGER))) \
         ORDER BY f.added_at DESC, f.rowid DESC",
    )
    .bind(uid)
    .fetch_all(pool)
    .await?)
}

/// Favoriten, neueste zuerst.
pub async fn list(pool: &SqlitePool, uid: i64) -> AppResult<Vec<PlayableTrack>> {
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT key FROM favorites WHERE user_id = ? ORDER BY added_at DESC, rowid DESC",
    )
    .bind(uid)
    .fetch_all(pool)
            .await?;
    Ok(playable::resolve_present(pool, keys)
        .await?
        .into_iter()
        .map(|(_, t)| t)
        .collect())
}

/// Eine Wiedergabe zählen: Verlauf, bei lokalen Titeln auch `play_count`/`last_played`.
pub async fn record_play(pool: &SqlitePool, uid: i64, r: &TrackRef) -> AppResult<()> {
    let key = playable::accept(pool, r).await?.ok_or(AppError::NotFound)?;
    let at = now();
    if let Some(Key::Local(id)) = Key::parse(&key) {
        library::record_play(pool, id, at).await?;
    }
    sqlx::query(
        "INSERT INTO history (user_id, key, played_at, play_count, seq) \
         VALUES (?, ?, ?, 1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM history)) \
         ON CONFLICT (user_id, key) DO UPDATE SET played_at = excluded.played_at, \
            play_count = history.play_count + 1, seq = excluded.seq",
    )
    .bind(uid)
    .bind(key)
    .bind(at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Zuletzt gespielt, neueste zuerst, jeder Titel einmal.
pub async fn history(pool: &SqlitePool, uid: i64, limit: u64) -> AppResult<Vec<PlayableTrack>> {
    // Etwas mehr holen – verwaiste Einträge fallen beim Auflösen heraus.
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT key FROM history WHERE user_id = ? ORDER BY seq DESC LIMIT ?",
    )
    .bind(uid)
    .bind((limit + 50) as i64)
        .fetch_all(pool)
        .await?;
    let mut items: Vec<PlayableTrack> = playable::resolve_present(pool, keys)
        .await?
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    items.truncate(limit as usize);
    Ok(items)
}
