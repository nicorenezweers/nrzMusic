//! Die eine Datenbank: SQLite (`data/nrzmusic.db`). Darin liegen Bibliothek (Metadaten der
//! Dateien im Musikordner), Playlists, Favoriten, Verlauf, Einstellungen, Download-Aufträge
//! und die Spotify-Anmeldung samt Abgleich.

pub mod downloads;
pub mod favorites;
pub mod listens;
pub mod library;
mod migrations;
pub mod playable;
pub mod playlists;
pub mod settings;
pub mod shares;
pub mod spotify;
pub mod users;

use crate::config::Config;
use crate::error::AppError;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::time::Duration;

/// Öffnet (bzw. erstellt) die Datenbank.
pub async fn open(config: &Config) -> Result<SqlitePool, AppError> {
    std::fs::create_dir_all(&config.data_dir)?;
    let options = SqliteConnectOptions::new()
        .filename(config.database_path())
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));
    Ok(SqlitePoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect_with(options)
        .await?)
}

/// Schema auf den neuesten Stand bringen. Die Migrationen sind Rust-Module ([`migrations`]);
/// sqlx merkt sich in `_sqlx_migrations`, was schon gelaufen ist.
pub async fn migrate(pool: &SqlitePool) -> Result<(), AppError> {
    migrations::migrator()
        .run(pool)
        .await
        .map_err(|e| AppError::Config(format!("Migration fehlgeschlagen: {e}")))
}

/// Öffnen und migrieren in einem Schritt.
pub async fn connect(config: &Config) -> Result<SqlitePool, AppError> {
    let pool = open(config).await?;
    if let Err(e) = migrate(&pool).await {
        pool.close().await;
        return Err(e);
    }
    Ok(pool)
}

/// Jetzt in Unix-Sekunden.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
