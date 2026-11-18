//! YouTube Music (`/api/v1/yt/*`): dünne Hülle um `nrzmusic_ytmusic::YtMusic`.
//! 503, solange YouTube Music ausgeschaltet ist (Umgebung oder Einstellungen), 502, wenn
//! YouTube nicht wie erwartet antwortet, 404, wenn es etwas nicht gibt.

use crate::db::playlists;
use crate::db::settings::SettingsStore;
use crate::error::{AppError, AppResult};
use crate::library::Events;
use nrzmusic_types::{
    ImportRequest, PlaylistSummary, ServerEvent, TrackRef, YtAlbum, YtArtist, YtPlaylist,
    YtSearchResults, YtShelf, YtTrack,
};
use nrzmusic_ytmusic::{playlist_id_from_url, SearchFilter, YtMusic};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{get, post, routes, Route, State};
use sqlx::SqlitePool;
use std::sync::Arc;

pub fn routes() -> Vec<Route> {
    routes![search, suggest, home, album, playlist, artist, radio, import]
}

/// Ist YouTube Music an?
fn enabled(store: &SettingsStore) -> AppResult<()> {
    if store.youtube_enabled() {
        Ok(())
    } else {
        Err(AppError::YoutubeOff)
    }
}

#[get("/yt/search?<q>&<filter>")]
async fn search(
    q: Option<&str>,
    filter: Option<&str>,
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
) -> AppResult<Json<YtSearchResults>> {
    enabled(store)?;
    let filter = match filter.map(str::trim).filter(|f| !f.is_empty()) {
        None => None,
        Some(f) => Some(SearchFilter::parse(f).ok_or_else(|| {
            AppError::bad("filter: erlaubt sind songs, videos, albums, artists, playlists.")
        })?),
    };
    let q = q.map(str::trim).unwrap_or("");
    if q.is_empty() {
        return Ok(Json(YtSearchResults::default()));
    }
    Ok(Json(yt.search(q, filter).await?))
}

#[get("/yt/suggest?<q>")]
async fn suggest(
    q: Option<&str>,
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
) -> AppResult<Json<Vec<String>>> {
    enabled(store)?;
    let q = q.map(str::trim).unwrap_or("");
    if q.is_empty() {
        return Ok(Json(Vec::new()));
    }
    Ok(Json(yt.suggestions(q).await?))
}

#[get("/yt/home")]
async fn home(
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
) -> AppResult<Json<Vec<YtShelf>>> {
    enabled(store)?;
    Ok(Json(yt.home().await?))
}

#[get("/yt/album/<browse_id>")]
async fn album(
    browse_id: &str,
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
) -> AppResult<Json<YtAlbum>> {
    enabled(store)?;
    Ok(Json(yt.album(browse_id).await?))
}

#[get("/yt/playlist/<playlist_id>")]
async fn playlist(
    playlist_id: &str,
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
) -> AppResult<Json<YtPlaylist>> {
    enabled(store)?;
    Ok(Json(yt.playlist(playlist_id).await?))
}

#[get("/yt/artist/<channel_id>")]
async fn artist(
    channel_id: &str,
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
) -> AppResult<Json<YtArtist>> {
    enabled(store)?;
    Ok(Json(yt.artist(channel_id).await?))
}

#[get("/yt/radio/<video_id>")]
async fn radio(
    video_id: &str,
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
) -> AppResult<Json<Vec<YtTrack>>> {
    enabled(store)?;
    Ok(Json(yt.radio(video_id).await?))
}

/// YouTube-(Music-)Playlist als eigene Playlist übernehmen (Titel als YouTube-Verweise).
#[post("/yt/import", data = "<body>")]
async fn import(
    body: Json<ImportRequest>,
    me: crate::auth::CurrentUser,
    yt: &State<Arc<YtMusic>>,
    store: &State<SettingsStore>,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<(Status, Json<PlaylistSummary>)> {
    enabled(store)?;
    let url = body.url.trim();
    let id = (!url.is_empty())
        .then(|| playlist_id_from_url(url))
        .flatten()
        .ok_or_else(|| AppError::bad("Das ist kein Link zu einer YouTube-Playlist."))?;
    let list = yt.playlist(&id).await?;
    let refs: Vec<TrackRef> = list.tracks.into_iter().map(TrackRef::Youtube).collect();
    let keys = super::playlists::accept_all(pool, &refs).await?;
    let name = if list.summary.title.trim().is_empty() {
        "YouTube-Playlist"
    } else {
        list.summary.title.trim()
    };
    let summary = playlists::create(
        pool,
        me.id(),
        name,
        list.description.as_deref().unwrap_or(""),
        Some(&id),
        &keys,
    )
    .await?;
    events.send_to(me.id(), ServerEvent::PlaylistsChanged);
    Ok((Status::Created, Json(summary)))
}
