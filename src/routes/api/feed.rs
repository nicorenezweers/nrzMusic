//! Feed ("Für dich"), Hörverhalten, Statistik und Einstellungen je Konto.

use crate::auth::CurrentUser;
use crate::db::listens;
use crate::db::settings::SettingsStore;
use crate::error::AppResult;
use crate::feed::Feed;
use nrzmusic_types::{DislikeRequest, FeedPage, FeedRequest, ListenReport, Stats, UserPrefs};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{delete, get, post, put, routes, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![page, dislike, undislike, listen, stats, prefs, save_prefs]
}

/// Nächste Seite des Feeds (ohne die schon gezeigten Titel).
#[post("/feed", data = "<body>")]
async fn page(
    body: Json<FeedRequest>,
    me: CurrentUser,
    feed: &State<Feed>,
    settings: &State<SettingsStore>,
) -> AppResult<Json<FeedPage>> {
    let exclude: Vec<String> = body.exclude.iter().rev().take(1000).cloned().collect();
    let count = body.count.unwrap_or(20) as usize;
    Ok(Json(feed.page(me.id(), &exclude, count, settings.youtube_enabled()).await?))
}

/// "Nicht interessiert" – Titel kommt nicht wieder, sein Künstler seltener.
#[post("/feed/dislike", data = "<body>")]
async fn dislike(body: Json<DislikeRequest>, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Status> {
    listens::dislike(pool, me.id(), &body.track).await?;
    Ok(Status::NoContent)
}

#[delete("/feed/dislike/<key>")]
async fn undislike(key: &str, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Status> {
    listens::undislike(pool, me.id(), key).await?;
    Ok(Status::NoContent)
}

/// Eine Wiedergabe ist vorbei (auch per `navigator.sendBeacon` beim Schliessen der Seite).
#[post("/listens", data = "<body>")]
async fn listen(body: Json<ListenReport>, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Status> {
    listens::record(pool, me.id(), &body).await?;
    Ok(Status::NoContent)
}

#[get("/stats?<days>&<tz>")]
async fn stats(days: Option<u32>, tz: Option<i64>, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Stats>> {
    let days = days.unwrap_or(30).min(3650);
    Ok(Json(listens::stats(pool, me.id(), days, tz.unwrap_or(0)).await?))
}

#[get("/prefs")]
async fn prefs(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<UserPrefs>> {
    Ok(Json(listens::prefs(pool, me.id()).await?))
}

#[put("/prefs", data = "<body>")]
async fn save_prefs(body: Json<UserPrefs>, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<UserPrefs>> {
    listens::save_prefs(pool, me.id(), &body).await?;
    Ok(Json(listens::prefs(pool, me.id()).await?))
}
