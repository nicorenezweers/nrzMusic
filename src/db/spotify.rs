//! Spotify: die Anmeldung (Tokens, zwischengespeicherter Benutzer) in `spotify_auth` und der
//! Abgleich Spotify-Titel → YouTube Music in `spotify_matches`.

use crate::db::now;
use crate::error::AppResult;
use nrzmusic_spotify::Tokens;
use nrzmusic_types::{SpotifyUser, YtTrack};
use sqlx::SqlitePool;
use std::collections::HashMap;

/// Ein Fehlschlag ("nichts gefunden") gilt so lange (Sekunden), danach wird neu gesucht.
pub const MISS_TTL: i64 = 7 * 24 * 60 * 60;

pub async fn tokens(pool: &SqlitePool, uid: i64) -> AppResult<Option<Tokens>> {
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT access_token, refresh_token, expires_at, scope FROM spotify_auth WHERE user_id = ?",
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?;
    Ok(
        row.map(|(access_token, refresh_token, expires_at, scope)| Tokens {
            access_token,
            refresh_token,
            expires_at,
            scope,
        }),
    )
}

/// Tokens speichern (der zwischengespeicherte Benutzer bleibt, bis er ersetzt wird).
pub async fn save_tokens(pool: &SqlitePool, uid: i64, t: &Tokens) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO spotify_auth (user_id, access_token, refresh_token, expires_at, scope, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (user_id) DO UPDATE SET access_token = excluded.access_token, \
            refresh_token = excluded.refresh_token, expires_at = excluded.expires_at, \
            scope = excluded.scope, updated_at = excluded.updated_at",
    )
    .bind(uid)
    .bind(&t.access_token)
    .bind(&t.refresh_token)
    .bind(t.expires_at)
    .bind(&t.scope)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Neue Anmeldung: Tokens und Benutzer ersetzen.
pub async fn save_login(
    pool: &SqlitePool,
    uid: i64,
    t: &Tokens,
    user: Option<&SpotifyUser>,
) -> AppResult<()> {
    save_tokens(pool, uid, t).await?;
    save_user(pool, uid, user).await
}

pub async fn save_user(pool: &SqlitePool, uid: i64, user: Option<&SpotifyUser>) -> AppResult<()> {
    sqlx::query("UPDATE spotify_auth SET user = ? WHERE user_id = ?")
        .bind(user.map(|u| serde_json::to_string(u).unwrap_or_default()))
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn user(pool: &SqlitePool, uid: i64) -> AppResult<Option<SpotifyUser>> {
    let data: Option<Option<String>> =
        sqlx::query_scalar("SELECT user FROM spotify_auth WHERE user_id = ?")
            .bind(uid)
            .fetch_optional(pool)
            .await?;
    Ok(data
        .flatten()
        .and_then(|d| serde_json::from_str::<SpotifyUser>(&d).ok()))
}

/// Abmelden: Tokens und Benutzer löschen. `true`, wenn es eine Anmeldung gab.
pub async fn delete_auth(pool: &SqlitePool, uid: i64) -> AppResult<bool> {
    let res = sqlx::query("DELETE FROM spotify_auth WHERE user_id = ?")
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Alle Anmeldungen über die eigene Spotify-App löschen (andere Client-ID); die über das
/// Webplayer-Cookie bleiben. `true`, wenn es welche gab.
pub async fn delete_all_auth(pool: &SqlitePool) -> AppResult<bool> {
    let res = sqlx::query("DELETE FROM spotify_auth WHERE scope <> ?")
        .bind(nrzmusic_spotify::WEB_PLAYER_SCOPE)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Eintrag im Abgleich: `Some(track)` = gefunden, `None` = nichts gefunden (noch gültig).
pub type Cached = Option<YtTrack>;

/// Abgeglichene Titel zu Spotify-IDs. Abgelaufene Fehlschläge fehlen in der Map.
pub async fn matches(pool: &SqlitePool, ids: &[&str]) -> AppResult<HashMap<String, Cached>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, Option<String>, i64)> = sqlx::query_as(
        "SELECT spotify_id, data, matched_at FROM spotify_matches \
         WHERE spotify_id IN (SELECT value FROM json_each(?))",
    )
    .bind(serde_json::to_string(ids).unwrap_or_default())
    .fetch_all(pool)
    .await?;
    let fresh_after = now() - MISS_TTL;
    let mut out = HashMap::with_capacity(rows.len());
    for (id, data, at) in rows {
        match data.map(|d| serde_json::from_str::<YtTrack>(&d)) {
            Some(Ok(track)) => {
                out.insert(id, Some(track));
            }
            Some(Err(e)) => tracing::warn!(spotify = %id, "Gespeicherter Abgleich unlesbar: {e}"),
            None if at >= fresh_after => {
                out.insert(id, None);
            }
            None => {}
        }
    }
    Ok(out)
}

/// Ergebnis eines Abgleichs speichern (Treffer oder Fehlschlag).
pub async fn save_match(
    pool: &SqlitePool,
    spotify_id: &str,
    track: Option<&YtTrack>,
    score: f64,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO spotify_matches (spotify_id, video_id, data, score, matched_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (spotify_id) DO UPDATE SET video_id = excluded.video_id, \
            data = excluded.data, score = excluded.score, matched_at = excluded.matched_at",
    )
    .bind(spotify_id)
    .bind(track.map(|t| t.video_id.as_str()))
    .bind(track.map(|t| serde_json::to_string(t).unwrap_or_default()))
    .bind(score)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}
