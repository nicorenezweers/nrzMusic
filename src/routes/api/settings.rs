//! Einstellungen (YouTube Music an/aus, Sprache, Region, Downloads, Spotify).

use crate::db::settings::{self, SettingsStore, DOWNLOAD_FORMATS};
use crate::downloads::Downloads;
use crate::error::{AppError, AppResult};
use crate::spotify::SpotifyService;
use nrzmusic_types::Settings;
use nrzmusic_ytmusic::YtMusic;
use rocket::serde::json::Json;
use rocket::{get, put, routes, Route, State};
use sqlx::SqlitePool;
use std::sync::Arc;

pub fn routes() -> Vec<Route> {
    routes![get, put]
}

#[get("/settings")]
fn get(store: &State<SettingsStore>) -> Json<Settings> {
    Json(store.get())
}

/// Sprache wie "de", "en-GB", "zh-Hant"; Region wie "CH", "US".
fn valid_code(s: &str, max: usize) -> bool {
    (2..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Redirect-URI: leer (Standard) oder eine http(s)-Adresse ohne Leerzeichen.
fn valid_redirect(s: &str) -> bool {
    s.is_empty()
        || ((s.starts_with("http://") || s.starts_with("https://"))
            && s.len() <= 512
            && s.len() > "https://".len()
            && !s.chars().any(|c| c.is_whitespace() || c.is_control()))
}

#[put("/settings", data = "<body>")]
async fn put(
    body: Json<Settings>,
    _admin: crate::auth::Admin,
    pool: &State<SqlitePool>,
    store: &State<SettingsStore>,
    yt: &State<Arc<YtMusic>>,
    spotify: &State<SpotifyService>,
    downloads: &State<Downloads>,
) -> AppResult<Json<Settings>> {
    let mut s = body.into_inner();
    s.youtube_language = s.youtube_language.trim().to_string();
    s.youtube_region = s.youtube_region.trim().to_ascii_uppercase();
    s.download_format = s.download_format.trim().to_ascii_lowercase();
    s.spotify_client_id = s.spotify_client_id.trim().to_string();
    s.spotify_redirect_uri = s.spotify_redirect_uri.trim().to_string();
    if !valid_code(&s.youtube_language, 12) {
        return Err(AppError::bad("Ungültige Sprache für YouTube Music."));
    }
    if !valid_code(&s.youtube_region, 3) {
        return Err(AppError::bad("Ungültige Region für YouTube Music."));
    }
    if !DOWNLOAD_FORMATS.contains(&s.download_format.as_str()) {
        return Err(AppError::bad(
            "Ungültiges Download-Format – erlaubt sind mp3, m4a und opus.",
        ));
    }
    if s.spotify_client_id.len() > 64
        || !s
            .spotify_client_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric())
    {
        return Err(AppError::bad("Ungültige Spotify-Client-ID."));
    }
    if !valid_redirect(&s.spotify_redirect_uri) {
        return Err(AppError::bad(
            "Ungültige Redirect-URI – erwartet wird eine Adresse mit http:// oder https://.",
        ));
    }
    // Die Voreinstellung aus der Umgebung nicht festschreiben, solange nichts anderes drinsteht
    let previous = store.get();
    let mut saved = s.clone();
    if store.spotify_client_id_is_preset() && s.spotify_client_id == previous.spotify_client_id {
        saved.spotify_client_id = String::new();
    }
    settings::save(pool, &saved).await?;
    yt.set_locale(&s.youtube_language, &s.youtube_region);
    store.set(saved);
    spotify.configure().await?;
    downloads.settings_changed();
    Ok(Json(store.get()))
}
