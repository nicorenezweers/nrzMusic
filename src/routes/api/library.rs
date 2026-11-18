//! Bibliothek: Titel, Alben, Künstler, lokale Suche und "neu scannen".

use crate::db::library::{self as store, AlbumScope, AlbumSort, TrackSort};
use crate::error::{AppError, AppResult};
use crate::library::Library;
use nrzmusic_types::{Album, AlbumDetail, Artist, ArtistDetail, LibrarySearch, Page, Track};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{get, post, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![tracks, track, albums, album, artists, artist, search, rescan]
}

/// `order=asc|desc`, sonst die Voreinstellung der Sortierung.
fn descending(order: Option<&str>, default_desc: bool) -> AppResult<bool> {
    match order.map(str::trim).filter(|o| !o.is_empty()) {
        None => Ok(default_desc),
        Some("asc") => Ok(false),
        Some("desc") => Ok(true),
        Some(_) => Err(AppError::bad("order: erlaubt sind asc und desc.")),
    }
}

#[get("/tracks?<q>&<sort>&<order>&<offset>&<limit>")]
async fn tracks(
    q: Option<&str>,
    sort: Option<&str>,
    order: Option<&str>,
    offset: Option<u64>,
    limit: Option<u64>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<Page<Track>>> {
    let sort = match sort.map(str::trim).filter(|s| !s.is_empty()) {
        None => TrackSort::Title,
        Some(s) => TrackSort::parse(s).ok_or_else(|| {
            AppError::bad("sort: erlaubt sind title, artist, album, added, played, duration.")
        })?,
    };
    let desc = descending(order, sort.default_desc())?;
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    Ok(Json(
        store::list(pool, q, sort, desc, offset.unwrap_or(0), limit).await?,
    ))
}

#[get("/tracks/<id>")]
async fn track(id: i64, pool: &State<SqlitePool>) -> AppResult<Json<Track>> {
    Ok(Json(store::get(pool, id).await?.ok_or(AppError::NotFound)?))
}

#[get("/albums?<q>&<sort>&<order>")]
async fn albums(
    q: Option<&str>,
    sort: Option<&str>,
    order: Option<&str>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<Vec<Album>>> {
    let sort = match sort.map(str::trim).filter(|s| !s.is_empty()) {
        None => AlbumSort::Title,
        Some(s) => AlbumSort::parse(s)
            .ok_or_else(|| AppError::bad("sort: erlaubt sind title, artist, year, added."))?,
    };
    let desc = descending(order, sort.default_desc())?;
    Ok(Json(
        store::albums(pool, AlbumScope::All, q, sort, desc, None).await?,
    ))
}

#[get("/albums/<id>")]
async fn album(id: &str, pool: &State<SqlitePool>) -> AppResult<Json<AlbumDetail>> {
    Ok(Json(
        store::album(pool, id).await?.ok_or(AppError::NotFound)?,
    ))
}

#[get("/artists?<q>")]
async fn artists(q: Option<&str>, pool: &State<SqlitePool>) -> AppResult<Json<Vec<Artist>>> {
    Ok(Json(store::artists(pool, q, None).await?))
}

#[get("/artists/<id>")]
async fn artist(id: &str, pool: &State<SqlitePool>) -> AppResult<Json<ArtistDetail>> {
    Ok(Json(
        store::artist(pool, id).await?.ok_or(AppError::NotFound)?,
    ))
}

/// Lokale Suche über Titel, Alben und Künstler.
#[get("/search?<q>&<limit>")]
async fn search(
    q: Option<&str>,
    limit: Option<u64>,
    pool: &State<SqlitePool>,
) -> AppResult<Json<LibrarySearch>> {
    let q = q.map(str::trim).unwrap_or("");
    if q.is_empty() {
        return Ok(Json(LibrarySearch::default()));
    }
    let limit = limit.unwrap_or(20).clamp(1, 200);
    Ok(Json(LibrarySearch {
        tracks: store::list(pool, Some(q), TrackSort::Played, true, 0, limit)
            .await?
            .items,
        albums: store::albums(
            pool,
            AlbumScope::All,
            Some(q),
            AlbumSort::Title,
            false,
            Some(limit),
        )
        .await?,
        artists: store::artists(pool, Some(q), Some(limit)).await?,
    }))
}

/// Voller Scan im Hintergrund (Fortschritt über `/events` und `/status`).
#[post("/library/rescan")]
fn rescan(_admin: crate::auth::Admin, library: &State<Library>) -> Status {
    library.start_full_scan();
    Status::Accepted
}
