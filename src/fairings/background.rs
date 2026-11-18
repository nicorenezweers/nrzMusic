//! Startet beim Hochfahren (Liftoff) die Hintergrundarbeiten: die Downloads (yt-dlp
//! einrichten, Arbeiter – auch in Tests), Überwachung des Musikordners, den ersten vollen Scan
//! und – je nach `scan_interval` – regelmässige volle Scans als Sicherheitsnetz. Alle enden mit
//! dem Server.

use crate::config::Config;
use crate::downloads::Downloads;
use crate::library::{watcher, Library};
use rocket::fairing::{Fairing, Info, Kind};
use rocket::{Orbit, Rocket};
use std::time::Duration;

pub struct Background;

#[rocket::async_trait]
impl Fairing for Background {
    fn info(&self) -> Info {
        Info {
            name: "Hintergrundarbeiten",
            kind: Kind::Liftoff,
        }
    }

    async fn on_liftoff(&self, rocket: &Rocket<Orbit>) {
        let (Some(library), Some(config)) = (
            rocket.state::<Library>().cloned(),
            rocket.state::<Config>().cloned(),
        ) else {
            return;
        };
        if let Some(downloads) = rocket.state::<Downloads>() {
            downloads.start(rocket.shutdown(), config.background);
        }
        if !config.background {
            return;
        }
        tracing::info!("Musikordner: {}", library.root().display());

        // Erst überwachen, dann scannen – was während des Scans dazukommt, geht nicht verloren.
        watcher::spawn(library.clone(), rocket.shutdown());
        library.start_full_scan();

        if config.genre_lookup {
            if let Some(pool) = rocket.state::<sqlx::SqlitePool>() {
                crate::genres::spawn(pool.clone(), rocket.shutdown());
            }
        }

        if config.scan_interval > 0 {
            let every = Duration::from_secs(config.scan_interval * 60);
            let mut shutdown = rocket.shutdown();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(every) => {}
                        _ = &mut shutdown => break,
                    }
                    if let Err(e) = library.full_scan().await {
                        tracing::warn!("Periodischer Scan: {e}");
                    }
                }
            });
        }
    }
}
