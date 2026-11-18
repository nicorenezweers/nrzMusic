//! Favoriten und Verlauf – lokale und YouTube-Titel gemischt.

use crate::auth::CurrentUser;
use crate::db::favorites;
use crate::error::AppResult;
use crate::library::Events;
use nrzmusic_types::{PlayableTrack, ServerEvent, TrackRef};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{delete, get, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![list, keys, add, remove, record, history]
}

/// Favoriten, neueste zuerst.
#[get("/favorites")]
async fn list(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Vec<PlayableTrack>>> {
    Ok(Json(favorites::list(pool, me.id()).await?))
}

/// Alle Schlüssel (`local:<id>`, `yt:<videoId>`) – für die Herz-Symbole.
#[get("/favorites/keys")]
async fn keys(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Vec<String>>> {
    Ok(Json(favorites::keys(pool, me.id()).await?))
}

#[put("/favorites", data = "<track>")]
async fn add(
    track: Json<TrackRef>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
    downloads: &State<crate::downloads::Downloads>,
) -> AppResult<Status> {
    favorites::add(pool, me.id(), &track).await?;
    events.send_to(me.id(), ServerEvent::FavoritesChanged);
    // Beim Liken gleich herunterladen (Einstellung des Kontos); Fehler stören das Herz nicht
    if let TrackRef::Youtube(t) = &*track {
        if crate::db::listens::prefs(pool, me.id()).await.is_ok_and(|p| p.auto_download_likes)
            && downloads.ensure_usable().await.is_ok()
        {
            if let Err(e) = downloads.enqueue(std::slice::from_ref(t)).await {
                tracing::warn!(video = %t.video_id, "Download beim Liken: {e}");
            }
        }
    }
    Ok(Status::NoContent)
}

#[delete("/favorites/<key>")]
async fn remove(key: &str, me: CurrentUser, pool: &State<SqlitePool>, events: &State<Events>) -> AppResult<Status> {
    if favorites::remove(pool, me.id(), key).await? {
        events.send_to(me.id(), ServerEvent::FavoritesChanged);
    }
    Ok(Status::NoContent)
}

/// Eine Wiedergabe zählen.
#[post("/history", data = "<track>")]
async fn record(track: Json<TrackRef>, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Status> {
    favorites::record_play(pool, me.id(), &track).await?;
    Ok(Status::NoContent)
}

/// Zuletzt gespielt, neueste zuerst, ohne Doppelte.
#[get("/history?<limit>")]
async fn history(
    limit: Option<u64>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
) -> AppResult<Json<Vec<PlayableTrack>>> {
    let limit = limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(favorites::history(pool, me.id(), limit).await?))
}
