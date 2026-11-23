//! JSON-API unter `/api/v1` – Vertrag siehe ARCHITECTURE.md und `nrzmusic-types`.

mod auth;
mod collection;
mod downloads;
mod events;
mod feed;
mod library;
pub mod media;
mod playlists;
pub mod presence;
mod settings;
mod shares;
mod spotify;
pub mod upload;
mod yt;
pub mod ytstream;

use crate::db::settings::SettingsStore;
use crate::db::{favorites, library as store};
use crate::error::AppResult;
use crate::library::Library;
use nrzmusic_types::{HomeData, LibraryStatus};
use rocket::serde::json::{json, Json, Value};
use rocket::{get, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    let mut r = routes![health, status, home];
    r.extend(auth::routes());
    r.extend(collection::routes());
    r.extend(downloads::routes());
    r.extend(events::routes());
    r.extend(feed::routes());
    r.extend(library::routes());
    r.extend(media::routes());
    r.extend(playlists::routes());
    r.extend(presence::routes());
    r.extend(settings::routes());
    r.extend(shares::routes());
    r.extend(spotify::routes());
    r.extend(upload::routes());
    r.extend(yt::routes());
    r.extend(ytstream::routes());
    r
}

/// Lebenszeichen für `nrzmusic healthcheck` und Monitoring.
#[get("/health")]
fn health() -> Value {
    json!({ "ok": true })
}

#[get("/status")]
async fn status(
    library: &State<Library>,
    settings: &State<SettingsStore>,
) -> AppResult<Json<LibraryStatus>> {
    let stats = store::stats(library.pool()).await?;
    Ok(Json(LibraryStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        music_dir: library.display_root().to_string(),
        tracks: stats.tracks,
        albums: stats.albums,
        artists: stats.artists,
        total_duration_ms: stats.duration_ms,
        total_size: stats.size,
        scanning: library.is_scanning(),
        scan_progress: library.progress().filter(|_| library.is_scanning()),
        last_scan: library.last_scan(),
        watching: library.is_watching(),
        youtube_enabled: settings.youtube_enabled(),
    }))
}

/// Startseite: zuletzt hinzugefügte Alben, zuletzt gespielt, meistgespielt, zufällige Alben.
#[get("/home")]
async fn home(me: crate::auth::CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<HomeData>> {
    use store::{AlbumScope, AlbumSort};
    Ok(Json(HomeData {
        recently_added: store::albums(
            pool,
            AlbumScope::All,
            None,
            AlbumSort::Added,
            true,
            Some(12),
        )
        .await?,
        recently_played: favorites::history(pool, me.id(), 12).await?,
        most_played: store::most_played(pool, 12).await?,
        random_albums: store::albums(
            pool,
            AlbumScope::All,
            None,
            AlbumSort::Random,
            false,
            Some(12),
        )
        .await?,
    }))
}
