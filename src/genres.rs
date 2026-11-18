//! Genres für die Statistik. Lokale Titel bringen ihr Genre in den Tags mit, YouTube-Titel
//! nicht – für deren Künstler fragt nrzMusic im Hintergrund die iTunes-Suche von Apple
//! (ohne Schlüssel; geschickt wird nur der Künstlername) und merkt sich das Ergebnis in
//! `artist_genres`. Langsam (ein Künstler alle paar Sekunden), damit niemand gedrosselt wird.
//! Abschaltbar mit `NRZMUSIC_GENRE_LOOKUP=false`.

use crate::db::listens;
use rocket::Shutdown;
use serde_json::Value;
use sqlx::SqlitePool;
use std::time::Duration;

pub fn spawn(pool: SqlitePool, mut shutdown: Shutdown) {
    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("nrzMusic")
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Genre-Suche: {e}");
                return;
            }
        };
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                _ = &mut shutdown => break,
            }
            let artists = match listens::artists_without_genre(&pool, 10).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::debug!("Genre-Suche: {e}");
                    continue;
                }
            };
            for artist in artists {
                match lookup(&http, &artist).await {
                    Ok(genre) => {
                        let _ = listens::set_artist_genre(&pool, &artist, genre.as_deref()).await;
                    }
                    // Netz weg oder gedrosselt: später noch einmal
                    Err(e) => {
                        tracing::debug!(artist, "Genre-Suche: {e}");
                        break;
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(4)) => {}
                    _ = &mut shutdown => return,
                }
            }
        }
    });
}

/// Hauptgenre eines Künstlers; `Ok(None)` = nichts gefunden.
async fn lookup(http: &reqwest::Client, artist: &str) -> Result<Option<String>, String> {
    let resp = http
        .get("https://itunes.apple.com/search")
        .query(&[("term", artist), ("entity", "musicArtist"), ("limit", "5")])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(pick(&v, artist))
}

fn pick(v: &Value, artist: &str) -> Option<String> {
    let results = v["results"].as_array()?;
    let exact = results
        .iter()
        .find(|r| r["artistName"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(artist)));
    exact
        .or(results.first())
        .and_then(|r| r["primaryGenreName"].as_str())
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_exact_artist() {
        let v: Value = serde_json::from_str(
            r#"{"results":[{"artistName":"Other","primaryGenreName":"Pop"},{"artistName":"Kraftwerk","primaryGenreName":"Electronic"}]}"#,
        )
        .unwrap();
        assert_eq!(pick(&v, "kraftwerk").as_deref(), Some("Electronic"));
        assert_eq!(pick(&v, "nobody").as_deref(), Some("Pop"));
        assert_eq!(pick(&serde_json::json!({"results": []}), "x"), None);
    }
}
