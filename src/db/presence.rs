//! Jetzt läuft: Einstellung je Konto (Sync an/aus, öffentlich an/aus, geheimes Token) und der
//! letzte gemeldete Stand. Geräte melden bei jeder Änderung und alle paar Sekunden, solange sie
//! spielen; daraus rechnen App, Widget und MCP die Position auf die Sekunde hoch.

use crate::db::shares::valid_token;
use crate::error::{AppError, AppResult};
use nrzmusic_types::{NowPlaying, NowPlayingReport, PresenceSettings, PublicNowPlaying};
use sqlx::SqlitePool;

/// Meldet ein spielendes Gerät so lange nichts mehr, gilt es als weg (pausiert).
pub const STALE_MS: i64 = 45_000;
/// Älter als das: nichts mehr zeigen.
pub const FORGET_MS: i64 = 12 * 3600 * 1000;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn new_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

struct Row {
    enabled: bool,
    public: bool,
    token: String,
    state: Option<String>,
}

async fn row(pool: &SqlitePool, uid: i64) -> AppResult<Option<Row>> {
    let r: Option<(i64, i64, String, Option<String>)> =
        sqlx::query_as("SELECT enabled, public, token, state FROM presence WHERE user_id = ?")
            .bind(uid)
            .fetch_optional(pool)
            .await?;
    Ok(r.map(|(e, p, token, state)| Row { enabled: e != 0, public: p != 0, token, state }))
}

fn settings_of(r: Option<Row>) -> PresenceSettings {
    let Some(r) = r else { return PresenceSettings::default() };
    let public = r.enabled && r.public;
    let t = &r.token;
    PresenceSettings {
        enabled: r.enabled,
        public,
        widget_path: public.then(|| format!("/widget/{t}")),
        api_path: public.then(|| format!("/api/v1/public/now-playing/{t}")),
        mcp_path: public.then(|| format!("/mcp/{t}")),
    }
}

pub async fn settings(pool: &SqlitePool, uid: i64) -> AppResult<PresenceSettings> {
    Ok(settings_of(row(pool, uid).await?))
}

/// Ein- oder ausschalten. Ausschalten vergisst auch den letzten Stand.
pub async fn update(pool: &SqlitePool, uid: i64, enabled: bool, public: bool) -> AppResult<PresenceSettings> {
    let public = enabled && public;
    sqlx::query(
        "INSERT INTO presence (user_id, enabled, public, token) VALUES (?, ?, ?, ?)
         ON CONFLICT (user_id) DO UPDATE SET enabled = excluded.enabled, public = excluded.public",
    )
    .bind(uid)
    .bind(enabled as i64)
    .bind(public as i64)
    .bind(new_token())
    .execute(pool)
    .await?;
    if !enabled {
        sqlx::query("UPDATE presence SET state = NULL, updated_at = 0 WHERE user_id = ?")
            .bind(uid)
            .execute(pool)
            .await?;
    }
    settings(pool, uid).await
}

/// Neue geheime Adresse – die alte funktioniert nicht mehr.
pub async fn rotate(pool: &SqlitePool, uid: i64) -> AppResult<PresenceSettings> {
    let n = sqlx::query("UPDATE presence SET token = ? WHERE user_id = ?")
        .bind(new_token())
        .bind(uid)
        .execute(pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    settings(pool, uid).await
}

fn parse_state(s: Option<&str>) -> Option<NowPlaying> {
    serde_json::from_str(s?).ok()
}

/// Letzter Stand, sofern eingeschaltet und nicht uralt; ein spielendes Gerät, das sich lange
/// nicht gemeldet hat, gilt als pausiert (an der hochgerechneten Stelle).
fn fresh(mut np: NowPlaying, now: i64) -> Option<NowPlaying> {
    if now - np.updated_at > FORGET_MS {
        return None;
    }
    if np.playing && now - np.updated_at > STALE_MS {
        np.position_ms = np.position_at(np.updated_at + STALE_MS);
        np.playing = false;
    }
    Some(np)
}

pub async fn current(pool: &SqlitePool, uid: i64) -> AppResult<Option<NowPlaying>> {
    let Some(r) = row(pool, uid).await?.filter(|r| r.enabled) else { return Ok(None) };
    Ok(parse_state(r.state.as_deref()).and_then(|np| fresh(np, now_ms())))
}

/// Meldung eines Geräts übernehmen. `Ok(None)`: Sync ist aus. `Ok(Some(None))`: verworfen
/// (ein anderes Gerät spielt gerade, dieses ist nur pausiert). Sonst der neue Stand.
pub async fn report(pool: &SqlitePool, uid: i64, rep: NowPlayingReport) -> AppResult<Option<Option<NowPlaying>>> {
    let Some(r) = row(pool, uid).await?.filter(|r| r.enabled) else { return Ok(None) };
    let now = now_ms();
    let before = parse_state(r.state.as_deref()).and_then(|np| fresh(np, now));
    let other_playing = before.as_ref().is_some_and(|b| b.playing && b.device_id != rep.device_id);
    if !rep.playing && other_playing {
        return Ok(Some(None));
    }
    let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let Some(track) = rep.track else {
        // Gerät hat nichts mehr geladen: nur löschen, wenn es das aktuelle war
        if before.as_ref().is_some_and(|b| b.device_id == rep.device_id) {
            store(pool, uid, None, now).await?;
            return Ok(Some(None));
        }
        return Ok(Some(None));
    };
    let duration_ms = if rep.duration_ms > 0 { rep.duration_ms } else { track.duration_ms().unwrap_or(0) };
    let np = NowPlaying {
        device_id: clip(&rep.device_id, 64),
        device_name: clip(rep.device_name.trim(), 60),
        device_kind: clip(&rep.device_kind, 16),
        track,
        position_ms: rep.position_ms,
        duration_ms,
        playing: rep.playing,
        updated_at: now,
    };
    store(pool, uid, Some(&np), now).await?;
    Ok(Some(Some(np)))
}

async fn store(pool: &SqlitePool, uid: i64, np: Option<&NowPlaying>, now: i64) -> AppResult<()> {
    let json = np.and_then(|n| serde_json::to_string(n).ok());
    sqlx::query("UPDATE presence SET state = ?, updated_at = ? WHERE user_id = ?")
        .bind(json)
        .bind(now)
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}

/// Öffentlicher Stand über das Token (`base` = Adresse des Servers für absolute Bild-URLs).
/// Unbekannt, ausgeschaltet oder nicht öffentlich → 404.
pub async fn public(pool: &SqlitePool, token: &str, base: &str) -> AppResult<PublicNowPlaying> {
    if !valid_token(token) {
        return Err(AppError::NotFound);
    }
    let r: Option<(i64, Option<String>, String)> = sqlx::query_as(
        "SELECT p.user_id, p.state, COALESCE(NULLIF(u.display_name, ''), u.username) FROM presence p JOIN users u ON u.id = p.user_id
         WHERE p.token = ? AND p.enabled = 1 AND p.public = 1",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    let Some((_, state, user)) = r else { return Err(AppError::NotFound) };
    let now = now_ms();
    let mut out = PublicNowPlaying { user, server_time: now, ..Default::default() };
    let Some(np) = parse_state(state.as_deref()).and_then(|np| fresh(np, now)) else { return Ok(out) };
    let t = &np.track;
    out.active = true;
    out.playing = np.playing;
    out.title = t.title().to_string();
    out.artist = t.artist();
    out.album = t.album().map(str::to_string);
    out.artwork = t.artwork_url(300).map(|a| if a.starts_with('/') { format!("{base}{a}") } else { a });
    out.link = match t {
        nrzmusic_types::PlayableTrack::Youtube(y) => Some(format!("https://music.youtube.com/watch?v={}", y.video_id)),
        _ => None,
    };
    out.position_ms = np.position_at(now);
    out.duration_ms = np.duration_ms;
    out.device_name = np.device_name;
    out.device_kind = np.device_kind;
    out.updated_at = np.updated_at;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nrzmusic_types::{PlayableTrack, YtTrack};

    fn np(playing: bool, at: i64) -> NowPlaying {
        NowPlaying {
            device_id: "a".into(),
            device_name: "Mac".into(),
            device_kind: "computer".into(),
            track: PlayableTrack::Youtube(YtTrack {
                video_id: "x".into(),
                title: "T".into(),
                artists: vec![],
                album: None,
                duration_ms: Some(200_000),
                thumbnail: None,
                explicit: false,
                is_video: false,
            }),
            position_ms: 10_000,
            duration_ms: 200_000,
            playing,
            updated_at: at,
        }
    }

    #[test]
    fn position_runs_while_playing_and_stops_when_stale() {
        assert_eq!(np(true, 1_000).position_at(6_000), 15_000);
        assert_eq!(np(false, 1_000).position_at(6_000), 10_000);
        assert_eq!(np(true, 0).position_at(10_000_000), 200_000, "nie über die Länge");
        let s = fresh(np(true, 0), STALE_MS + 60_000).unwrap();
        assert!(!s.playing);
        assert_eq!(s.position_ms, 10_000 + STALE_MS as u64);
        assert!(fresh(np(false, 0), FORGET_MS + 1).is_none());
    }
}
