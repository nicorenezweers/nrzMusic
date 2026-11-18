//! Geteilte Links: ein Titel, die Likes oder eine Playlist eines Kontos unter `/s/<token>`.
//! Der Link zeigt immer den aktuellen Stand; über ihn lassen sich genau diese Titel abspielen.

use crate::db::playable::{self, Key};
use crate::db::{favorites, now, playlists, users};
use crate::error::{AppError, AppResult};
use nrzmusic_types::{PlayableTrack, ShareInfo, ShareKind, ShareRequest, SharedView};
use sqlx::SqlitePool;

fn kind_str(k: ShareKind) -> &'static str {
    match k {
        ShareKind::Track => "track",
        ShareKind::Favorites => "favorites",
        ShareKind::Playlist => "playlist",
    }
}

fn parse_kind(s: &str) -> Option<ShareKind> {
    match s {
        "track" => Some(ShareKind::Track),
        "favorites" => Some(ShareKind::Favorites),
        "playlist" => Some(ShareKind::Playlist),
        _ => None,
    }
}

/// Zufälliges Token (16 Byte, hex) – nicht zu erraten.
fn new_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Gültige Token-Form (schützt die Abfragen vor Unsinn).
pub fn valid_token(t: &str) -> bool {
    t.len() == 32 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

struct Row {
    token: String,
    user_id: i64,
    kind: ShareKind,
    target: String,
    created_at: i64,
    views: i64,
}

async fn by_token(pool: &SqlitePool, token: &str) -> AppResult<Option<Row>> {
    if !valid_token(token) {
        return Ok(None);
    }
    let r: Option<(String, i64, String, String, i64, i64)> = sqlx::query_as(
        "SELECT token, user_id, kind, target, created_at, views FROM shares WHERE token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    Ok(r.and_then(|(token, user_id, kind, target, created_at, views)| {
        Some(Row { token, user_id, kind: parse_kind(&kind)?, target, created_at, views })
    }))
}

/// Link anlegen; gibt es ihn für dasselbe schon, kommt derselbe zurück.
pub async fn create(pool: &SqlitePool, uid: i64, req: &ShareRequest) -> AppResult<ShareInfo> {
    let target = match req.kind {
        ShareKind::Track => {
            let r = req.track.as_ref().ok_or_else(|| AppError::bad("track fehlt."))?;
            playable::accept(pool, r).await?.ok_or(AppError::NotFound)?
        }
        ShareKind::Favorites => String::new(),
        ShareKind::Playlist => {
            let id = req.playlist_id.ok_or_else(|| AppError::bad("playlist_id fehlt."))?;
            // Nur eigene Playlists (sonst 404)
            playlists::get_summary(pool, uid, id).await?;
            id.to_string()
        }
    };
    let kind = kind_str(req.kind);
    sqlx::query(
        "INSERT INTO shares (token, user_id, kind, target, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (user_id, kind, target) DO NOTHING",
    )
    .bind(new_token())
    .bind(uid)
    .bind(kind)
    .bind(&target)
    .bind(now())
    .execute(pool)
    .await?;
    let token: String =
        sqlx::query_scalar("SELECT token FROM shares WHERE user_id = ? AND kind = ? AND target = ?")
            .bind(uid)
            .bind(kind)
            .bind(&target)
            .fetch_one(pool)
            .await?;
    let row = by_token(pool, &token).await?.ok_or(AppError::NotFound)?;
    info(pool, row).await
}

async fn title_of(pool: &SqlitePool, row: &Row) -> AppResult<String> {
    Ok(match row.kind {
        ShareKind::Favorites => String::new(),
        ShareKind::Track => playable::resolve(pool, std::slice::from_ref(&row.target))
            .await?
            .pop()
            .flatten()
            .map(|t| format!("{} – {}", t.title(), t.artist()))
            .unwrap_or_default(),
        ShareKind::Playlist => match row.target.parse::<i64>() {
            Ok(id) => playlists::get_summary(pool, row.user_id, id).await.map(|p| p.name).unwrap_or_default(),
            Err(_) => String::new(),
        },
    })
}

async fn info(pool: &SqlitePool, row: Row) -> AppResult<ShareInfo> {
    let title = title_of(pool, &row).await?;
    Ok(ShareInfo {
        path: format!("/s/{}", row.token),
        token: row.token,
        kind: row.kind,
        title,
        created_at: row.created_at,
        views: row.views.max(0) as u64,
    })
}

/// Eigene Links, neueste zuerst.
pub async fn list(pool: &SqlitePool, uid: i64) -> AppResult<Vec<ShareInfo>> {
    let tokens: Vec<String> =
        sqlx::query_scalar("SELECT token FROM shares WHERE user_id = ? ORDER BY created_at DESC, rowid DESC")
            .bind(uid)
            .fetch_all(pool)
            .await?;
    let mut out = Vec::with_capacity(tokens.len());
    for t in tokens {
        if let Some(row) = by_token(pool, &t).await? {
            out.push(info(pool, row).await?);
        }
    }
    Ok(out)
}

/// Link löschen (nur eigene).
pub async fn delete(pool: &SqlitePool, uid: i64, token: &str) -> AppResult<()> {
    let res = sqlx::query("DELETE FROM shares WHERE token = ? AND user_id = ?")
        .bind(token)
        .bind(uid)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// Der Inhalt eines Links (ohne Anmeldung). `count_view` zählt einen Aufruf.
pub async fn view(pool: &SqlitePool, token: &str, count_view: bool) -> AppResult<SharedView> {
    let row = by_token(pool, token).await?.ok_or(AppError::NotFound)?;
    let owner = users::get(pool, row.user_id)
        .await?
        .map(|a| a.label().to_string())
        .unwrap_or_default();
    let (title, description, tracks) = match row.kind {
        ShareKind::Track => {
            let t = playable::resolve(pool, std::slice::from_ref(&row.target))
                .await?
                .pop()
                .flatten()
                .ok_or(AppError::NotFound)?;
            (t.title().to_string(), t.artist(), vec![t])
        }
        ShareKind::Favorites => (String::new(), String::new(), favorites::list(pool, row.user_id).await?),
        ShareKind::Playlist => {
            let id: i64 = row.target.parse().map_err(|_| AppError::NotFound)?;
            let p = playlists::get(pool, row.user_id, id).await?;
            (p.summary.name, p.summary.description, p.items)
        }
    };
    if count_view {
        sqlx::query("UPDATE shares SET views = views + 1 WHERE token = ?")
            .bind(&row.token)
            .execute(pool)
            .await?;
    }
    let artwork = tracks.iter().find_map(|t| t.artwork_url(600));
    Ok(SharedView { token: row.token, kind: row.kind, title, description, owner, artwork, tracks })
}

/// Gehört der Titel `key` zu diesem Link? Dann der Titel (zum Abspielen über den Link).
pub async fn shared_track(pool: &SqlitePool, token: &str, key: &str) -> AppResult<PlayableTrack> {
    if Key::parse(key).is_none() {
        return Err(AppError::NotFound);
    }
    let v = view(pool, token, false).await?;
    v.tracks.into_iter().find(|t| t.key() == key).ok_or(AppError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let t = new_token();
        assert!(valid_token(&t));
        assert!(!valid_token("abc"));
        assert!(!valid_token(&"g".repeat(32)));
        assert_eq!(parse_kind(kind_str(ShareKind::Playlist)), Some(ShareKind::Playlist));
    }
}
