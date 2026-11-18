//! Playlists: Name, Beschreibung und eine geordnete Liste von Titel-Schlüsseln.
//!
//! Positionen in der API meinen die sichtbaren Einträge (`Playlist.items`, Index = Position).
//! Verwaiste lokale Einträge (Datei gelöscht – ihre ID wird nie neu vergeben) werden nicht
//! angezeigt und beim nächsten Umsortieren/Entfernen endgültig weggeräumt.

use crate::db::{now, playable};
use crate::error::{AppError, AppResult};
use nrzmusic_types::{cover_url, PlayableTrack, Playlist, PlaylistSummary, PlaylistUpdate};
use sqlx::SqlitePool;

#[derive(sqlx::FromRow)]
struct PlaylistRow {
    id: i64,
    name: String,
    description: String,
    youtube_playlist_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

/// Playlist des Kontos `uid` (fremde gibt es für ihn nicht: 404).
async fn row(pool: &SqlitePool, uid: i64, id: i64) -> AppResult<PlaylistRow> {
    sqlx::query_as(
        "SELECT id, name, description, youtube_playlist_id, created_at, updated_at \
         FROM playlists WHERE id = ? AND user_id = ?",
    )
    .bind(id)
    .bind(uid)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)
}

async fn stored_keys(pool: &SqlitePool, id: i64) -> AppResult<Vec<String>> {
    Ok(
        sqlx::query_scalar(
            "SELECT key FROM playlist_items WHERE playlist_id = ? ORDER BY position",
        )
        .bind(id)
        .fetch_all(pool)
        .await?,
    )
}

/// Sichtbare Einträge in Reihenfolge.
async fn items(pool: &SqlitePool, id: i64) -> AppResult<Vec<(String, PlayableTrack)>> {
    playable::resolve_present(pool, stored_keys(pool, id).await?).await
}

fn summary(row: PlaylistRow, items: &[(String, PlayableTrack)]) -> PlaylistSummary {
    let mut artwork: Vec<String> = Vec::new();
    for (_, t) in items {
        if artwork.len() == 4 {
            break;
        }
        let url = match t {
            PlayableTrack::Local(t) => t.cover.as_deref().map(|c| cover_url(c, 300)),
            PlayableTrack::Youtube(t) => t.thumbnail.clone(),
        };
        if let Some(url) = url.filter(|u| !artwork.contains(u)) {
            artwork.push(url);
        }
    }
    PlaylistSummary {
        id: row.id,
        name: row.name,
        description: row.description,
        item_count: items.len() as u32,
        duration_ms: items.iter().filter_map(|(_, t)| t.duration_ms()).sum(),
        artwork,
        youtube_playlist_id: row.youtube_playlist_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Alle Playlists, zuletzt geänderte zuerst.
pub async fn list(pool: &SqlitePool, uid: i64) -> AppResult<Vec<PlaylistSummary>> {
    let rows: Vec<PlaylistRow> = sqlx::query_as(
        "SELECT id, name, description, youtube_playlist_id, created_at, updated_at \
         FROM playlists WHERE user_id = ? ORDER BY updated_at DESC, id DESC",
    )
    .bind(uid)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let items = items(pool, r.id).await?;
        out.push(summary(r, &items));
    }
    Ok(out)
}

pub async fn get(pool: &SqlitePool, uid: i64, id: i64) -> AppResult<Playlist> {
    let r = row(pool, uid, id).await?;
    let items = items(pool, id).await?;
    let summary = summary(r, &items);
    Ok(Playlist {
        summary,
        items: items.into_iter().map(|(_, t)| t).collect(),
    })
}

pub async fn get_summary(pool: &SqlitePool, uid: i64, id: i64) -> AppResult<PlaylistSummary> {
    let r = row(pool, uid, id).await?;
    let items = items(pool, id).await?;
    Ok(summary(r, &items))
}

fn clean_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::bad("Die Playlist braucht einen Namen."));
    }
    Ok(name.chars().take(200).collect())
}

/// Neue Playlist mit (bereits geprüften) Schlüsseln.
pub async fn create(
    pool: &SqlitePool,
    uid: i64,
    name: &str,
    description: &str,
    youtube_playlist_id: Option<&str>,
    keys: &[String],
) -> AppResult<PlaylistSummary> {
    let name = clean_name(name)?;
    let at = now();
    let mut tx = pool.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO playlists (user_id, name, description, youtube_playlist_id, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(uid)
    .bind(name)
    .bind(description.trim())
    .bind(youtube_playlist_id)
    .bind(at)
    .bind(at)
    .fetch_one(&mut *tx)
    .await?;
    insert_items(&mut tx, id, 0, keys).await?;
    tx.commit().await?;
    get_summary(pool, uid, id).await
}

async fn insert_items(
    tx: &mut sqlx::SqliteConnection,
    id: i64,
    start: i64,
    keys: &[String],
) -> AppResult<()> {
    for (i, key) in keys.iter().enumerate() {
        sqlx::query("INSERT INTO playlist_items (playlist_id, position, key) VALUES (?, ?, ?)")
            .bind(id)
            .bind(start + i as i64)
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

async fn touch(tx: &mut sqlx::SqliteConnection, id: i64) -> AppResult<()> {
    sqlx::query("UPDATE playlists SET updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

pub async fn update(
    pool: &SqlitePool,
    uid: i64,
    id: i64,
    upd: &PlaylistUpdate,
) -> AppResult<PlaylistSummary> {
    row(pool, uid, id).await?;
    let mut tx = pool.begin().await?;
    if let Some(name) = &upd.name {
        sqlx::query("UPDATE playlists SET name = ? WHERE id = ?")
            .bind(clean_name(name)?)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(description) = &upd.description {
        sqlx::query("UPDATE playlists SET description = ? WHERE id = ?")
            .bind(description.trim())
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    touch(&mut tx, id).await?;
    tx.commit().await?;
    get_summary(pool, uid, id).await
}

pub async fn delete(pool: &SqlitePool, uid: i64, id: i64) -> AppResult<()> {
    let res = sqlx::query("DELETE FROM playlists WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(uid)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// Anhängen.
pub async fn append(
    pool: &SqlitePool,
    uid: i64,
    id: i64,
    keys: &[String],
) -> AppResult<PlaylistSummary> {
    row(pool, uid, id).await?;
    let mut tx = pool.begin().await?;
    let next: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_items WHERE playlist_id = ?",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    insert_items(&mut tx, id, next, keys).await?;
    touch(&mut tx, id).await?;
    tx.commit().await?;
    get_summary(pool, uid, id).await
}

/// Einträge komplett neu schreiben (nur die sichtbaren, in der neuen Reihenfolge).
async fn rewrite(
    pool: &SqlitePool,
    uid: i64,
    id: i64,
    keys: &[String],
) -> AppResult<PlaylistSummary> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM playlist_items WHERE playlist_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    insert_items(&mut tx, id, 0, keys).await?;
    touch(&mut tx, id).await?;
    tx.commit().await?;
    get_summary(pool, uid, id).await
}

/// Eintrag an einer (sichtbaren) Position entfernen.
pub async fn remove_at(
    pool: &SqlitePool,
    uid: i64,
    id: i64,
    position: usize,
) -> AppResult<PlaylistSummary> {
    row(pool, uid, id).await?;
    let mut keys: Vec<String> = items(pool, id).await?.into_iter().map(|(k, _)| k).collect();
    if position >= keys.len() {
        return Err(AppError::NotFound);
    }
    keys.remove(position);
    rewrite(pool, uid, id, &keys).await
}

/// Neue Reihenfolge als Liste der alten Positionen (muss jede Position genau einmal enthalten).
pub async fn reorder(
    pool: &SqlitePool,
    uid: i64,
    id: i64,
    order: &[u32],
) -> AppResult<PlaylistSummary> {
    row(pool, uid, id).await?;
    let keys: Vec<String> = items(pool, id).await?.into_iter().map(|(k, _)| k).collect();
    let mut seen = vec![false; keys.len()];
    if order.len() != keys.len()
        || !order.iter().all(|&p| {
            let p = p as usize;
            p < seen.len() && !std::mem::replace(&mut seen[p], true)
        })
    {
        return Err(AppError::bad(
            "Die neue Reihenfolge muss jede Position genau einmal enthalten.",
        ));
    }
    let reordered: Vec<String> = order.iter().map(|&p| keys[p as usize].clone()).collect();
    rewrite(pool, uid, id, &reordered).await
}

/// Playlist des Kontos, die aus dieser Spotify-Quelle übernommen wurde.
pub async fn by_spotify(pool: &SqlitePool, uid: i64, spotify_id: &str) -> AppResult<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM playlists WHERE user_id = ? AND spotify_id = ? ORDER BY id LIMIT 1",
    )
    .bind(uid)
    .bind(spotify_id)
    .fetch_optional(pool)
    .await?)
}

/// Aus Spotify übernehmen: vorhandene Playlist (gleiche Quelle) neu füllen, sonst anlegen.
pub async fn upsert_spotify(
    pool: &SqlitePool,
    uid: i64,
    spotify_id: &str,
    name: &str,
    description: &str,
    keys: &[String],
) -> AppResult<PlaylistSummary> {
    if let Some(id) = by_spotify(pool, uid, spotify_id).await? {
        sqlx::query("UPDATE playlists SET name = ?, description = ? WHERE id = ?")
            .bind(clean_name(name)?)
            .bind(description.trim())
            .bind(id)
            .execute(pool)
            .await?;
        return rewrite(pool, uid, id, keys).await;
    }
    let summary = create(pool, uid, name, description, None, keys).await?;
    sqlx::query("UPDATE playlists SET spotify_id = ? WHERE id = ?")
        .bind(spotify_id)
        .bind(summary.id)
        .execute(pool)
        .await?;
    Ok(summary)
}
