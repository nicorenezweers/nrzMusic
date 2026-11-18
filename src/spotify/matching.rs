//! Abgleich Spotify → YouTube Music: erst mit der ISRC suchen (findet meist genau die
//! Aufnahme), sonst mit "Künstler Titel" (`match_query`); der beste Kandidat nach
//! `match_score` gewinnt, wenn er `MATCH_THRESHOLD` erreicht.
//!
//! Die Suche steckt hinter [`SongSearch`], damit Tests ohne YouTube auskommen.

use nrzmusic_spotify::{match_query, match_score, MATCH_THRESHOLD};
use nrzmusic_types::{SpTrack, YtTrack};
use nrzmusic_ytmusic::{SearchFilter, YtMusic};
use std::sync::Arc;

/// Suche nach Songs bei YouTube Music.
#[rocket::async_trait]
pub trait SongSearch: Send + Sync {
    async fn songs(&self, query: &str) -> Result<Vec<YtTrack>, String>;
}

/// Die echte Suche über `nrzmusic-ytmusic` (Filter "songs").
pub struct YtSongSearch(pub Arc<YtMusic>);

#[rocket::async_trait]
impl SongSearch for YtSongSearch {
    async fn songs(&self, query: &str) -> Result<Vec<YtTrack>, String> {
        self.0
            .search(query, Some(SearchFilter::Songs))
            .await
            .map(|r| r.songs)
            .map_err(|e| e.to_string())
    }
}

/// Sieht aus wie eine ISRC (z. B. "USUM71703861")?
fn valid_isrc(isrc: &str) -> bool {
    isrc.len() == 12 && isrc.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Bester Kandidat (≥ `MATCH_THRESHOLD`) in `candidates`, verglichen mit `best`.
fn consider(
    track: &SpTrack,
    candidates: Vec<YtTrack>,
    best: Option<(YtTrack, f64)>,
) -> Option<(YtTrack, f64)> {
    let mut best = best;
    for c in candidates {
        if !crate::downloads::ytdlp::valid_video_id(&c.video_id) {
            continue;
        }
        let score = match_score(track, &c);
        if score >= MATCH_THRESHOLD && best.as_ref().is_none_or(|(_, b)| score > *b) {
            best = Some((c, score));
        }
    }
    best
}

/// Besten Treffer suchen. `Ok(None)` = nichts Passendes; `Err` = YouTube nicht erreichbar
/// (dann wird nichts zwischengespeichert).
pub async fn best_match(
    search: &dyn SongSearch,
    track: &SpTrack,
) -> Result<Option<(YtTrack, f64)>, String> {
    let mut best = None;
    if let Some(isrc) = track
        .isrc
        .as_deref()
        .map(str::trim)
        .filter(|i| valid_isrc(i))
    {
        best = consider(track, search.songs(isrc).await?, best);
        if best.is_some() {
            return Ok(best);
        }
    }
    let query = match_query(track);
    if !query.is_empty() {
        best = consider(track, search.songs(&query).await?, best);
    }
    Ok(best)
}
