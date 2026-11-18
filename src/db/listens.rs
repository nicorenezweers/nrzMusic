//! Hörverhalten: jede Wiedergabe mit gehörter Zeit, Überspringen und Ende ("listens"),
//! "Nicht interessiert" (dislikes), Einstellungen je Konto und die Statistik daraus.

use crate::db::playable;
use crate::db::{library, now};
use crate::error::{AppError, AppResult};
use nrzmusic_types::{
    ListenReport, PlayableTrack, StatBucket, StatEntry, StatTrack, Stats, TrackRef, UserPrefs,
};
use sqlx::SqlitePool;

/// Höchste glaubwürdige Hördauer einer Wiedergabe (3 h) – schützt die Statistik vor Unsinn.
const MAX_LISTEN_MS: u64 = 3 * 3600 * 1000;
/// Ab so viel gehörter Zeit zählt eine Wiedergabe als "gespielt" (Statistik).
pub const PLAY_MS: i64 = 30_000;

/// Hauptkünstler, Kanal-ID und Genre eines Titels.
pub fn artist_of(t: &PlayableTrack) -> (String, Option<String>, Option<String>) {
    match t {
        PlayableTrack::Local(l) => {
            let a = if l.artist.trim().is_empty() { &l.album_artist } else { &l.artist };
            // "A feat. B", "A, B", "A & B" → A
            (main_artist(a), None, l.genre.clone().filter(|g| !g.trim().is_empty()))
        }
        PlayableTrack::Youtube(y) => {
            let first = y.artists.first();
            (
                first.map(|a| a.name.trim().to_string()).unwrap_or_default(),
                first.and_then(|a| a.id.clone()),
                None,
            )
        }
    }
}

/// Erster Künstler aus einer Angabe wie "A feat. B" oder "A, B" ("A & B" bleibt – oft ein Bandname).
pub fn main_artist(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut cut = s.len();
    for sep in [" feat. ", " feat ", " ft. ", " featuring ", " (feat", ", ", "; ", " / "] {
        if let Some(p) = lower.find(sep) {
            cut = cut.min(p);
        }
    }
    s[..cut].trim().to_string()
}

/// Wiedergabe aufzeichnen. Lokale Titel, die es nicht mehr gibt → 404.
pub async fn record(pool: &SqlitePool, uid: i64, r: &ListenReport) -> AppResult<()> {
    let key = playable::accept(pool, &r.track).await?.ok_or(AppError::NotFound)?;
    let track = match &r.track {
        TrackRef::Local { id } => PlayableTrack::Local(library::get(pool, *id).await?.ok_or(AppError::NotFound)?),
        TrackRef::Youtube(t) => PlayableTrack::Youtube(t.clone()),
    };
    let listened = r.listened_ms.min(MAX_LISTEN_MS) as i64;
    let duration = r.duration_ms.max(track.duration_ms().unwrap_or(0)).min(MAX_LISTEN_MS) as i64;
    let (artist, artist_id, genre) = artist_of(&track);
    let source: String = r.source.chars().filter(|c| c.is_ascii_alphanumeric()).take(16).collect();
    let started = now() - listened / 1000;
    sqlx::query(
        "INSERT INTO listens (user_id, key, title, artist, artist_id, genre, image, started_at, \
            listened_ms, duration_ms, skipped, completed, source) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uid)
    .bind(&key)
    .bind(track.title())
    .bind(&artist)
    .bind(artist_id)
    .bind(genre)
    .bind(track.artwork_url(300))
    .bind(started)
    .bind(listened)
    .bind(duration)
    .bind(r.skipped && !r.completed)
    .bind(r.completed)
    .bind(source)
    .execute(pool)
    .await?;
    Ok(())
}

/// "Nicht interessiert" merken.
pub async fn dislike(pool: &SqlitePool, uid: i64, r: &TrackRef) -> AppResult<String> {
    let key = playable::accept(pool, r).await?.ok_or(AppError::NotFound)?;
    let artist = match playable::resolve(pool, std::slice::from_ref(&key)).await?.pop().flatten() {
        Some(t) => artist_of(&t).0,
        None => String::new(),
    };
    sqlx::query(
        "INSERT INTO dislikes (user_id, key, artist, at) VALUES (?, ?, ?, ?) \
         ON CONFLICT (user_id, key) DO UPDATE SET at = excluded.at",
    )
    .bind(uid)
    .bind(&key)
    .bind(artist)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(key)
}

pub async fn undislike(pool: &SqlitePool, uid: i64, key: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM dislikes WHERE user_id = ? AND key = ?")
        .bind(uid)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

// ------------------------------------------------------------------ Einstellungen je Konto

pub async fn prefs(pool: &SqlitePool, uid: i64) -> AppResult<UserPrefs> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM user_prefs WHERE user_id = ?")
            .bind(uid)
            .fetch_all(pool)
            .await?;
    let mut p = UserPrefs::default();
    for (k, v) in rows {
        if k == "auto_download_likes" {
            p.auto_download_likes = v == "1";
        }
    }
    Ok(p)
}

pub async fn save_prefs(pool: &SqlitePool, uid: i64, p: &UserPrefs) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO user_prefs (user_id, key, value) VALUES (?, 'auto_download_likes', ?) \
         ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind(uid)
    .bind(if p.auto_download_likes { "1" } else { "0" })
    .execute(pool)
    .await?;
    Ok(())
}

// ------------------------------------------------------------------ Genres

/// Künstler aus dem Hörverhalten, deren Genre noch niemand nachgeschlagen hat
/// (YouTube-Titel bringen keins mit).
pub async fn artists_without_genre(pool: &SqlitePool, limit: i64) -> AppResult<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT l.artist FROM listens l \
         WHERE l.artist <> '' AND (l.genre IS NULL OR l.genre = '') \
           AND NOT EXISTS (SELECT 1 FROM artist_genres g WHERE g.artist = l.artist) \
         GROUP BY l.artist COLLATE NOCASE ORDER BY SUM(l.listened_ms) DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn set_artist_genre(pool: &SqlitePool, artist: &str, genre: Option<&str>) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO artist_genres (artist, genre, looked_up_at) VALUES (?, ?, ?) \
         ON CONFLICT (artist) DO UPDATE SET genre = excluded.genre, looked_up_at = excluded.looked_up_at",
    )
    .bind(artist)
    .bind(genre)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

// ------------------------------------------------------------------ Statistik

/// Tage seit 1970-01-01 für ein Datum (proleptisch gregorianisch).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn u(v: i64) -> u64 {
    v.max(0) as u64
}

/// Statistik über die letzten `days` Tage (0 = alles); `tz` Minuten wie `getTimezoneOffset()`.
pub async fn stats(pool: &SqlitePool, uid: i64, days: u32, tz: i64) -> AppResult<Stats> {
    let off = tz.clamp(-14 * 60, 14 * 60) * 60;
    let now = now();
    let since = if days == 0 { 0 } else { now - days as i64 * 86_400 };

    let (total, plays, skips, tracks, artists): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(listened_ms), 0), \
                COALESCE(SUM(listened_ms >= ? OR completed), 0), \
                COALESCE(SUM(skipped), 0), \
                COUNT(DISTINCT key), \
                COUNT(DISTINCT CASE WHEN artist <> '' THEN lower(artist) END) \
         FROM listens WHERE user_id = ? AND started_at >= ?",
    )
    .bind(PLAY_MS)
    .bind(uid)
    .bind(since)
    .fetch_one(pool)
    .await?;

    let first: Option<i64> = sqlx::query_scalar("SELECT MIN(started_at) FROM listens WHERE user_id = ?")
        .bind(uid)
        .fetch_one(pool)
        .await?;

    let rows: Vec<(String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT MAX(artist), SUM(listened_ms), SUM(listened_ms >= ? OR completed), MAX(image) \
         FROM listens WHERE user_id = ? AND started_at >= ? AND artist <> '' \
         GROUP BY lower(artist) ORDER BY 2 DESC LIMIT 12",
    )
    .bind(PLAY_MS)
    .bind(uid)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let top_artists = rows
        .into_iter()
        .map(|(name, ms, plays, image)| StatEntry { name, ms: u(ms), plays: u(plays), image })
        .collect();

    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT key, SUM(listened_ms), SUM(listened_ms >= ? OR completed) \
         FROM listens WHERE user_id = ? AND started_at >= ? \
         GROUP BY key ORDER BY 2 DESC LIMIT 15",
    )
    .bind(PLAY_MS)
    .bind(uid)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let keys: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let resolved = playable::resolve(pool, &keys).await?;
    let top_tracks = rows
        .into_iter()
        .zip(resolved)
        .filter_map(|((_, ms, plays), t)| t.map(|track| StatTrack { track, ms: u(ms), plays: u(plays) }))
        .collect();

    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT MAX(gen), SUM(ms), SUM(p) FROM ( \
            SELECT COALESCE(NULLIF(trim(l.genre), ''), g.genre) AS gen, l.listened_ms AS ms, \
                   (l.listened_ms >= ? OR l.completed) AS p \
            FROM listens l LEFT JOIN artist_genres g ON g.artist = l.artist \
            WHERE l.user_id = ? AND l.started_at >= ?) \
         WHERE gen IS NOT NULL AND gen <> '' GROUP BY lower(gen) ORDER BY 2 DESC LIMIT 12",
    )
    .bind(PLAY_MS)
    .bind(uid)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let top_genres = rows
        .into_iter()
        .map(|(name, ms, plays)| StatEntry { name, ms: u(ms), plays: u(plays), image: None })
        .collect();

    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT CAST(strftime('%H', started_at - ?, 'unixepoch') AS INTEGER), \
                CAST(strftime('%w', started_at - ?, 'unixepoch') AS INTEGER), SUM(listened_ms) \
         FROM listens WHERE user_id = ? AND started_at >= ? GROUP BY 1, 2",
    )
    .bind(off)
    .bind(off)
    .bind(uid)
    .bind(since)
    .fetch_all(pool)
    .await?;
    let mut by_hour = vec![0u64; 24];
    let mut by_weekday = vec![0u64; 7];
    for (h, w, ms) in rows {
        if let Some(x) = by_hour.get_mut(h as usize) {
            *x += u(ms);
        }
        // %w: 0 = Sonntag → Montag zuerst
        if let Some(x) = by_weekday.get_mut(((w + 6) % 7) as usize) {
            *x += u(ms);
        }
    }

    let local_now = now - off;
    let (bucket, timeline) = if days > 0 && days <= 62 {
        let rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT (started_at - ?) / 86400, SUM(listened_ms) FROM listens \
             WHERE user_id = ? AND started_at >= ? GROUP BY 1",
        )
        .bind(off)
        .bind(uid)
        .bind(since)
        .fetch_all(pool)
        .await?;
        let today = local_now.div_euclid(86_400);
        let first_day = today - days as i64 + 1;
        let map: std::collections::HashMap<i64, i64> = rows.into_iter().collect();
        let tl = (first_day..=today)
            .map(|d| StatBucket { start: d * 86_400, ms: u(*map.get(&d).unwrap_or(&0)) })
            .collect();
        ("day", tl)
    } else {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT strftime('%Y-%m', started_at - ?, 'unixepoch'), SUM(listened_ms) FROM listens \
             WHERE user_id = ? AND started_at >= ? GROUP BY 1",
        )
        .bind(off)
        .bind(uid)
        .bind(since)
        .fetch_all(pool)
        .await?;
        let parse = |s: &str| -> Option<(i64, i64)> {
            let (y, m) = s.split_once('-')?;
            Some((y.parse().ok()?, m.parse().ok()?))
        };
        let map: std::collections::HashMap<(i64, i64), i64> =
            rows.iter().filter_map(|(k, ms)| parse(k).map(|ym| (ym, *ms))).collect();
        let ym = |ts: i64| {
            let d = ts.div_euclid(86_400);
            // Rückrechnung Tag → Jahr/Monat
            let z = d + 719_468;
            let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
            (y, m)
        };
        let start_ts = if days == 0 { first.map(|f| f - off) } else { Some(since - off) };
        let mut tl = Vec::new();
        if let Some(st) = start_ts {
            let (mut y, mut m) = ym(st);
            let end = ym(local_now);
            while (y, m) <= end && tl.len() < 240 {
                tl.push(StatBucket {
                    start: days_from_civil(y, m, 1) * 86_400,
                    ms: u(*map.get(&(y, m)).unwrap_or(&0)),
                });
                m += 1;
                if m > 12 {
                    m = 1;
                    y += 1;
                }
            }
        }
        ("month", tl)
    };

    Ok(Stats {
        days,
        total_ms: u(total),
        plays: u(plays),
        skips: u(skips),
        tracks: u(tracks),
        artists: u(artists),
        top_artists,
        top_tracks,
        top_genres,
        by_hour,
        by_weekday,
        bucket: bucket.to_string(),
        timeline,
        since: first,
    })
}

// ------------------------------------------------------------------ Geschmack (für den Feed)

/// Eine Wiedergabe in Kurzform.
pub struct ListenRow {
    pub key: String,
    pub artist: String,
    pub artist_id: Option<String>,
    pub started_at: i64,
    pub listened_ms: i64,
    pub duration_ms: i64,
    pub skipped: bool,
    pub completed: bool,
}

/// Wiedergaben seit `since`, neueste zuerst (höchstens `limit`).
pub async fn recent(pool: &SqlitePool, uid: i64, since: i64, limit: i64) -> AppResult<Vec<ListenRow>> {
    type Row = (String, String, Option<String>, i64, i64, i64, bool, bool);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT key, artist, artist_id, started_at, listened_ms, duration_ms, skipped, completed \
         FROM listens WHERE user_id = ? AND started_at >= ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(uid)
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(key, artist, artist_id, started_at, listened_ms, duration_ms, skipped, completed)| ListenRow {
            key,
            artist,
            artist_id,
            started_at,
            listened_ms,
            duration_ms,
            skipped,
            completed,
        })
        .collect())
}

/// "Nicht interessiert": (Schlüssel, Künstler).
pub async fn dislikes(pool: &SqlitePool, uid: i64) -> AppResult<Vec<(String, String)>> {
    Ok(sqlx::query_as("SELECT key, artist FROM dislikes WHERE user_id = ?")
        .bind(uid)
        .fetch_all(pool)
        .await?)
}

/// Favoriten mit Zeitpunkt.
pub async fn favorites(pool: &SqlitePool, uid: i64) -> AppResult<Vec<(String, i64)>> {
    Ok(sqlx::query_as("SELECT key, added_at FROM favorites WHERE user_id = ? ORDER BY added_at DESC LIMIT 500")
        .bind(uid)
        .fetch_all(pool)
        .await?)
}

/// Alter Verlauf (vor 1.6, ohne Hördauer): Schlüssel, Zähler, zuletzt.
pub async fn history_counts(pool: &SqlitePool, uid: i64) -> AppResult<Vec<(String, i64, i64)>> {
    Ok(sqlx::query_as(
        "SELECT key, play_count, played_at FROM history WHERE user_id = ? ORDER BY seq DESC LIMIT 300",
    )
    .bind(uid)
    .fetch_all(pool)
    .await?)
}

/// Lokale Titel bestimmter Künstler (zufällig) bzw. irgendwelche selten gespielten.
pub async fn local_by_artists(pool: &SqlitePool, artists: &[String], limit: i64) -> AppResult<Vec<i64>> {
    if artists.is_empty() {
        return Ok(Vec::new());
    }
    let lower: Vec<String> = artists.iter().map(|a| a.to_lowercase()).collect();
    Ok(sqlx::query_scalar(
        "SELECT id FROM tracks WHERE lower(artist) IN (SELECT value FROM json_each(?)) \
            OR lower(album_artist) IN (SELECT value FROM json_each(?)) \
         ORDER BY RANDOM() LIMIT ?",
    )
    .bind(serde_json::to_string(&lower).unwrap_or_default())
    .bind(serde_json::to_string(&lower).unwrap_or_default())
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

pub async fn local_random(pool: &SqlitePool, limit: i64) -> AppResult<Vec<i64>> {
    Ok(sqlx::query_scalar("SELECT id FROM tracks ORDER BY play_count > 0, RANDOM() LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_artists() {
        assert_eq!(main_artist("Daft Punk feat. Pharrell"), "Daft Punk");
        assert_eq!(main_artist("A, B & C"), "A");
        assert_eq!(main_artist("Simon & Garfunkel"), "Simon & Garfunkel");
        assert_eq!(main_artist("Solo"), "Solo");
    }

    #[test]
    fn civil() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 1), 20_758);
    }
}
