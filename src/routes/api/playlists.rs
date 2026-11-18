//! Playlists mit lokalen und YouTube-Titeln.

use crate::auth::CurrentUser;
use crate::db::{playable, playlists};
use crate::error::AppResult;
use crate::library::Events;
use nrzmusic_types::{
    AddItems, NewPlaylist, Playlist, PlaylistSummary, PlaylistUpdate, Reorder, ServerEvent,
    TrackRef,
};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{delete, get, patch, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![
        list,
        create,
        get,
        update,
        remove,
        add_items,
        remove_item,
        reorder
    ]
}

/// Verweise übernehmen (YouTube-Metadaten speichern); nicht vorhandene lokale Titel fallen weg.
pub(crate) async fn accept_all(pool: &SqlitePool, refs: &[TrackRef]) -> AppResult<Vec<String>> {
    let mut keys = Vec::with_capacity(refs.len());
    for r in refs {
        if let Some(key) = playable::accept(pool, r).await? {
            keys.push(key);
        }
    }
    Ok(keys)
}

#[get("/playlists")]
async fn list(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Vec<PlaylistSummary>>> {
    Ok(Json(playlists::list(pool, me.id()).await?))
}

#[post("/playlists", data = "<body>")]
async fn create(
    body: Json<NewPlaylist>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<(Status, Json<PlaylistSummary>)> {
    let keys = accept_all(pool, &body.items).await?;
    let summary = playlists::create(pool, me.id(), &body.name, &body.description, None, &keys).await?;
    events.send_to(me.id(), ServerEvent::PlaylistsChanged);
    Ok((Status::Created, Json(summary)))
}

#[get("/playlists/<id>")]
async fn get(id: i64, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Playlist>> {
    Ok(Json(playlists::get(pool, me.id(), id).await?))
}

#[patch("/playlists/<id>", data = "<body>")]
async fn update(
    id: i64,
    body: Json<PlaylistUpdate>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<Json<PlaylistSummary>> {
    let summary = playlists::update(pool, me.id(), id, &body).await?;
    events.send_to(me.id(), ServerEvent::PlaylistsChanged);
    Ok(Json(summary))
}

#[delete("/playlists/<id>")]
async fn remove(id: i64, me: CurrentUser, pool: &State<SqlitePool>, events: &State<Events>) -> AppResult<Status> {
    playlists::delete(pool, me.id(), id).await?;
    events.send_to(me.id(), ServerEvent::PlaylistsChanged);
    Ok(Status::NoContent)
}

#[post("/playlists/<id>/items", data = "<body>")]
async fn add_items(
    id: i64,
    body: Json<AddItems>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<Json<PlaylistSummary>> {
    playlists::get_summary(pool, me.id(), id).await?;
    let keys = accept_all(pool, &body.items).await?;
    let summary = playlists::append(pool, me.id(), id, &keys).await?;
    events.send_to(me.id(), ServerEvent::PlaylistsChanged);
    Ok(Json(summary))
}

#[delete("/playlists/<id>/items/<position>")]
async fn remove_item(
    id: i64,
    position: usize,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<Json<PlaylistSummary>> {
    let summary = playlists::remove_at(pool, me.id(), id, position).await?;
    events.send_to(me.id(), ServerEvent::PlaylistsChanged);
    Ok(Json(summary))
}

#[put("/playlists/<id>/order", data = "<body>")]
async fn reorder(
    id: i64,
    body: Json<Reorder>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<Json<PlaylistSummary>> {
    let summary = playlists::reorder(pool, me.id(), id, &body.order).await?;
    events.send_to(me.id(), ServerEvent::PlaylistsChanged);
    Ok(Json(summary))
}
