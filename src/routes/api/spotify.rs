//! Spotify (`/api/v1/spotify/*`). 401, solange keine Anmeldung besteht (eigene Spotify-App
//! oder Webplayer-Cookie; ohne Client-ID und ohne Cookie 503) (oder Spotify sie widerrufen hat). Tokens verlassen den Server nie.

use crate::auth::CurrentUser;
use crate::error::{AppError, AppResult};
use crate::spotify::{import, SpotifyService, MAX_MATCH_TRACKS};
use nrzmusic_spotify::{parse_callback, CallbackParams};
use nrzmusic_types::{
    Page, SpImportRequest, SpMatchRequest, SpPlaylist, SpPlaylistSummary, SpTrack, SpotifyCallback,
    SpotifyCookie, SpotifyLogin, SpotifyStatus,
};
use rocket::http::{RawStr, Status};
use rocket::response::Redirect;
use rocket::serde::json::Json;
use rocket::{delete, get, post, routes, Route, State};

pub fn routes() -> Vec<Route> {
    routes![
        status,
        login,
        callback,
        callback_paste,
        connect_cookie,
        disconnect,
        liked,
        playlists,
        playlist,
        public_playlist,
        match_tracks,
        import_start
    ]
}

#[get("/spotify/status")]
async fn status(me: CurrentUser, spotify: &State<SpotifyService>) -> AppResult<Json<SpotifyStatus>> {
    Ok(Json(spotify.status(me.id()).await?))
}

#[post("/spotify/login")]
fn login(me: CurrentUser, spotify: &State<SpotifyService>) -> AppResult<Json<SpotifyLogin>> {
    Ok(Json(spotify.login(me.id())?))
}

/// Ziel der Weiterleitung von Spotify: Anmeldung abschliessen, dann zur Spotify-Seite der App.
#[get("/spotify/callback?<code>&<state>&<error>")]
async fn callback(
    code: Option<&str>,
    state: Option<&str>,
    error: Option<&str>,
    spotify: &State<SpotifyService>,
) -> Redirect {
    let params = match (error, code, state) {
        (Some(error), _, _) => Some(CallbackParams::Error {
            error: error.to_string(),
            state: state.map(str::to_string),
        }),
        (None, Some(code), Some(state)) if !code.is_empty() && !state.is_empty() => {
            Some(CallbackParams::Code {
                code: code.to_string(),
                state: state.to_string(),
            })
        }
        _ => None,
    };
    let result = match params {
        // Ohne Sitzung: das Konto kommt aus `state` (siehe SpotifyService::finish)
        Some(p) => spotify.finish(None, p).await,
        None => Err(AppError::bad(
            "Die Adresse enthält keinen Anmeldecode von Spotify.",
        )),
    };
    match result {
        Ok(()) => Redirect::to("/spotify"),
        Err(e) => {
            let msg = e.public_message();
            Redirect::to(format!(
                "/spotify?error={}",
                RawStr::new(&msg).percent_encode()
            ))
        }
    }
}

/// Die Adresse, auf die Spotify weitergeleitet hat, aus der Adresszeile eingefügt.
#[post("/spotify/callback", data = "<body>")]
async fn callback_paste(
    body: Json<SpotifyCallback>,
    me: CurrentUser,
    spotify: &State<SpotifyService>,
) -> AppResult<Json<SpotifyStatus>> {
    let params = parse_callback(&body.url).ok_or_else(|| {
        AppError::bad(
            "In der Adresse fehlen code und state – bitte die ganze Adresse nach der Anmeldung einfügen.",
        )
    })?;
    spotify.finish(Some(me.id()), params).await?;
    Ok(Json(spotify.status(me.id()).await?))
}

/// Mit dem Webplayer-Cookie `sp_dc` verbinden (ohne eigene Spotify-App, ohne Premium).
#[post("/spotify/cookie", data = "<body>")]
async fn connect_cookie(
    body: Json<SpotifyCookie>,
    me: CurrentUser,
    spotify: &State<SpotifyService>,
) -> AppResult<Json<SpotifyStatus>> {
    spotify.connect_cookie(me.id(), &body.cookie).await?;
    Ok(Json(spotify.status(me.id()).await?))
}

#[delete("/spotify/session")]
async fn disconnect(me: CurrentUser, spotify: &State<SpotifyService>) -> AppResult<Status> {
    spotify.disconnect(me.id()).await?;
    Ok(Status::NoContent)
}

#[get("/spotify/liked?<offset>&<limit>")]
async fn liked(
    offset: Option<u32>,
    limit: Option<u32>,
    me: CurrentUser,
    spotify: &State<SpotifyService>,
) -> AppResult<Json<Page<SpTrack>>> {
    let limit = limit.unwrap_or(50).clamp(1, 50);
    Ok(Json(spotify.liked(me.id(), offset.unwrap_or(0), limit).await?))
}

#[get("/spotify/playlists")]
async fn playlists(
    me: CurrentUser,
    spotify: &State<SpotifyService>,
) -> AppResult<Json<Vec<SpPlaylistSummary>>> {
    Ok(Json(spotify.playlists(me.id()).await?))
}

#[get("/spotify/playlists/<id>")]
async fn playlist(
    id: &str,
    me: CurrentUser,
    spotify: &State<SpotifyService>,
) -> AppResult<Json<SpPlaylist>> {
    Ok(Json(spotify.playlist(me.id(), id).await?))
}

/// Öffentliche Playlist ohne Spotify-Anmeldung (Einbettungsseite, höchstens 100 Titel),
/// `youtube` aus dem Abgleich-Zwischenspeicher.
#[get("/spotify/public/<id>")]
async fn public_playlist(
    id: &str,
    _me: CurrentUser,
    spotify: &State<SpotifyService>,
) -> AppResult<Json<SpPlaylist>> {
    let mut list = spotify.api().public_playlist(id).await?;
    spotify.fill_matches(&mut list.tracks).await?;
    Ok(Json(list))
}

/// Titel mit YouTube Music abgleichen (zwischengespeichert je Spotify-ID).
#[post("/spotify/match", data = "<body>")]
async fn match_tracks(
    body: Json<SpMatchRequest>,
    spotify: &State<SpotifyService>,
) -> AppResult<Json<Vec<SpTrack>>> {
    let tracks = body.into_inner().tracks;
    if tracks.len() > MAX_MATCH_TRACKS {
        return Err(AppError::bad(format!(
            "Höchstens {MAX_MATCH_TRACKS} Titel auf einmal."
        )));
    }
    Ok(Json(spotify.match_tracks(tracks, &|_, _| {}).await?))
}

/// Import im Hintergrund – Fortschritt als `SpotifyProgress`-Ereignisse.
#[post("/spotify/import", data = "<body>")]
async fn import_start(
    body: Json<SpImportRequest>,
    me: CurrentUser,
    spotify: &State<SpotifyService>,
) -> AppResult<Status> {
    import::start(spotify, me.id(), body.into_inner()).await?;
    Ok(Status::Accepted)
}
