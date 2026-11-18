//! nrzMusic – selbst gehosteter Musik-Client. Aufbau: siehe ARCHITECTURE.md.

pub mod auth;
pub mod config;
pub mod db;
pub mod downloads;
pub mod error;
pub mod fairings;
pub mod feed;
pub mod genres;
pub mod library;
pub mod routes;
pub mod spotify;

use rocket::figment::Figment;
use rocket::{Build, Rocket};
use std::sync::Arc;

/// Austauschbare Anbindungen nach aussen – in Tests ein Ersatz-Spotify
/// (`Spotify::with_endpoints`) und eine Ersatz-Suche bei YouTube Music.
#[derive(Default)]
pub struct Services {
    pub spotify: Option<nrzmusic_spotify::Spotify>,
    pub song_search: Option<Arc<dyn spotify::SongSearch>>,
}

/// Baut die Rocket-Instanz mit der normalen Konfiguration (`Rocket.toml`, `NRZMUSIC_*`).
pub async fn build() -> Result<Rocket<Build>, error::AppError> {
    build_with(config::figment()).await
}

/// Baut die Rocket-Instanz aus einer gegebenen Konfiguration (auch für Tests).
pub async fn build_with(figment: Figment) -> Result<Rocket<Build>, error::AppError> {
    build_with_services(figment, Services::default()).await
}

/// Baut die Rocket-Instanz aus Konfiguration und Anbindungen:
/// Ordner anlegen, Datenbank öffnen und migrieren, Zustand, Fairings, Routen, Catcher.
pub async fn build_with_services(
    figment: Figment,
    services: Services,
) -> Result<Rocket<Build>, error::AppError> {
    let config: config::Config = figment
        .extract()
        .map_err(|e| error::AppError::Config(e.to_string()))?;
    config.create_dirs().map_err(|e| {
        error::AppError::Config(format!(
            "Ordner {} / {} nicht anlegbar: {e}",
            config.data_dir.display(),
            config.music_dir.display()
        ))
    })?;
    let pool = db::connect(&config).await?;
    let auth = auth::Auth::new(pool.clone(), config.auth, &config.public_url);
    auth.init().await?;

    let settings = db::settings::load(&pool).await?;
    let ytmusic = Arc::new(nrzmusic_ytmusic::YtMusic::new());
    ytmusic.set_locale(&settings.youtube_language, &settings.youtube_region);
    let settings = db::settings::SettingsStore::new(settings, config.youtube)
        .with_env(config.downloads, &config.spotify_client_id);

    let events = library::Events::new();
    let library = library::Library::new(pool.clone(), &config, events.clone());
    let downloads = downloads::Downloads::new(
        pool.clone(),
        library.clone(),
        events.clone(),
        settings.clone(),
        &config,
    );
    let search = services
        .song_search
        .unwrap_or_else(|| Arc::new(spotify::YtSongSearch(ytmusic.clone())));
    let spotify = spotify::SpotifyService::new(
        Arc::new(services.spotify.unwrap_or_default()),
        pool.clone(),
        events.clone(),
        settings.clone(),
        search,
        downloads.clone(),
        config.default_spotify_redirect(),
    );
    spotify.configure().await?;
    let feed = feed::Feed::new(pool.clone(), ytmusic.clone());

    Ok(rocket::custom(figment)
        .manage(pool)
        .manage(auth)
        .manage(config)
        .manage(events)
        .manage(library)
        .manage(settings)
        .manage(ytmusic)
        .manage(downloads)
        .manage(spotify)
        .manage(feed)
        .attach(fairings::RequestLog)
        .attach(auth::Gate)
        .attach(fairings::SecurityHeaders)
        .attach(fairings::Background)
        .attach(fairings::CloseDatabase)
        .mount("/", routes::assets::routes())
        .mount("/", routes::spa::routes())
        .mount("/api/v1", routes::api::routes())
        .register("/", routes::catchers::all()))
}
