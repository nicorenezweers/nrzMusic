//! Jetzt läuft: Einstellung (`/presence`), Meldungen der Geräte (`/now-playing`) und der
//! öffentliche Stand fürs Widget (`/public/now-playing/<token>`, ohne Anmeldung, CORS offen).

use crate::auth::CurrentUser;
use crate::config::Config;
use crate::db::presence;
use crate::error::{AppError, AppResult};
use crate::library::Events;
use crate::routes::spa::HostHeader;
use nrzmusic_types::{NowPlaying, NowPlayingReport, PresenceSettings, PresenceUpdate, PublicNowPlaying, ServerEvent};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{get, post, put, routes, Responder, Route, State};
use sqlx::SqlitePool;

pub fn routes() -> Vec<Route> {
    routes![get_settings, put_settings, rotate, get_now, post_now, public_now]
}

#[get("/presence")]
async fn get_settings(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<PresenceSettings>> {
    Ok(Json(presence::settings(pool, me.id()).await?))
}

#[put("/presence", data = "<body>")]
async fn put_settings(
    body: Json<PresenceUpdate>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<Json<PresenceSettings>> {
    let s = presence::update(pool, me.id(), body.enabled, body.public).await?;
    if !s.enabled {
        events.send_to(me.id(), ServerEvent::NowPlaying { now: None, takeover: false });
    }
    Ok(Json(s))
}

/// Neue geheime Adresse für Widget, JSON und MCP.
#[post("/presence/rotate")]
async fn rotate(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<PresenceSettings>> {
    Ok(Json(presence::rotate(pool, me.id()).await?))
}

#[get("/now-playing")]
async fn get_now(me: CurrentUser, pool: &State<SqlitePool>) -> AppResult<Json<Option<NowPlaying>>> {
    // Auf jetzt umgerechnet: Geräte mit falsch gehender Uhr zeigen trotzdem die richtige Stelle
    let now = presence::now_ms();
    Ok(Json(presence::current(pool, me.id()).await?.map(|mut np| {
        np.position_ms = np.position_at(now);
        np.updated_at = now;
        np
    })))
}

/// Meldung eines Geräts; alle Geräte des Kontos bekommen den neuen Stand als Ereignis.
#[post("/now-playing", data = "<body>")]
async fn post_now(
    body: Json<NowPlayingReport>,
    me: CurrentUser,
    pool: &State<SqlitePool>,
    events: &State<Events>,
) -> AppResult<Status> {
    let rep = body.into_inner();
    if rep.device_id.trim().is_empty() {
        return Err(AppError::BadRequest("device_id fehlt.".into()));
    }
    let takeover = rep.takeover;
    let had_track = rep.track.is_some();
    match presence::report(pool, me.id(), rep).await? {
        None => Err(AppError::Forbidden("„Jetzt läuft“ ist in den Einstellungen ausgeschaltet.".into())),
        Some(Some(np)) => {
            events.send_to(me.id(), ServerEvent::NowPlaying { now: Some(np), takeover });
            Ok(Status::NoContent)
        }
        Some(None) => {
            if !had_track {
                let now = presence::current(pool, me.id()).await?;
                events.send_to(me.id(), ServerEvent::NowPlaying { now, takeover: false });
            }
            Ok(Status::NoContent)
        }
    }
}

/// JSON-Antwort, die jede Seite abrufen darf (Widgets auf fremden Seiten).
#[derive(Responder)]
pub struct Cors<T> {
    inner: T,
    origin: rocket::http::Header<'static>,
    cache: rocket::http::Header<'static>,
}

impl<T> Cors<T> {
    pub fn new(inner: T) -> Self {
        Cors {
            inner,
            origin: rocket::http::Header::new("Access-Control-Allow-Origin", "*"),
            cache: rocket::http::Header::new("Cache-Control", "no-store"),
        }
    }
}

/// Was das Konto gerade hört – ohne Anmeldung, über die geheime Adresse.
#[get("/public/now-playing/<token>")]
async fn public_now(
    token: &str,
    host: HostHeader,
    pool: &State<SqlitePool>,
    config: &State<Config>,
) -> AppResult<Cors<Json<PublicNowPlaying>>> {
    Ok(Cors::new(Json(presence::public(pool, token, &host.base(config)).await?)))
}
