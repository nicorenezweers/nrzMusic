//! Teilen: eigene Links verwalten (`/shares`, angemeldet) und der öffentliche Teil
//! (`/public/share/<token>…`, ohne Anmeldung): Inhalt lesen und genau dessen Titel abspielen.

use crate::auth::CurrentUser;
use crate::db::settings::SettingsStore;
use crate::db::shares;
use crate::downloads::Downloads;
use crate::error::AppResult;
use crate::library::Library;
use crate::routes::api::media::{audio_content_type, AudioFile, RangeHeader};
use crate::routes::api::ytstream::{self, YtAudio};
use nrzmusic_types::{PlayableTrack, ShareInfo, ShareRequest, SharedView};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{delete, get, post, routes, Responder, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![create, list, remove, public_view, public_stream]
}

#[post("/shares", data = "<body>")]
async fn create(body: Json<ShareRequest>, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<ShareInfo>> {
    Ok(Json(shares::create(pool, me.id(), &body).await?))
}

#[get("/shares")]
async fn list(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Vec<ShareInfo>>> {
    Ok(Json(shares::list(pool, me.id()).await?))
}

#[delete("/shares/<token>")]
async fn remove(token: &str, me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Status> {
    shares::delete(pool, me.id(), token).await?;
    Ok(Status::NoContent)
}

/// Inhalt eines Links – ohne Anmeldung.
#[get("/public/share/<token>")]
async fn public_view(token: &str, pool: &State<SqlitePool>) -> AppResult<Json<SharedView>> {
    Ok(Json(shares::view(pool, token, true).await?))
}

#[derive(Responder)]
enum SharedAudio {
    File(AudioFile),
    Youtube(YtAudio),
}

/// Tonspur eines Titels, der zum Link gehört – ohne Anmeldung, nur diese Titel.
#[get("/public/share/<token>/stream/<key>")]
async fn public_stream(
    token: &str,
    key: &str,
    range: RangeHeader,
    pool: &State<SqlitePool>,
    settings: &State<SettingsStore>,
    downloads: &State<Downloads>,
    library: &State<Library>,
) -> AppResult<SharedAudio> {
    match shares::shared_track(pool, token, key).await? {
        PlayableTrack::Local(t) => Ok(SharedAudio::File(
            AudioFile::open(library.abs_path(&t.path), audio_content_type(&t.format), range.0.as_deref(), None).await?,
        )),
        PlayableTrack::Youtube(t) => Ok(SharedAudio::Youtube(
            ytstream::stream(&t.video_id, range, settings, downloads, library).await?,
        )),
    }
}
