//! YouTube-Titel direkt abspielen, ohne IFrame-Player: yt-dlp sucht die Adresse der besten
//! Tonspur (M4A/AAC bevorzugt – das spielen alle Browser und Android), nrzMusic merkt sie
//! sich bis kurz vor ihrem Ablauf und reicht die Bytes an den Browser durch (mit `Range`,
//! damit Spulen geht). Durchreichen ist nötig, weil YouTubes Adressen an die IP des Servers
//! gebunden sind. Werbung gibt es so keine – es kommt nur die Tonspur.

use super::{ytdlp, Downloads};
use crate::error::{AppError, AppResult};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Zeitlimit für das Auflösen einer Adresse.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);
/// So lange gilt ein Video als dauerhaft nicht verfügbar (spart yt-dlp-Aufrufe beim Vorladen).
const GONE_TTL: Duration = Duration::from_secs(3600);

/// Videos, die es bei YouTube nicht (mehr) gibt: Video-ID → (seit, Meldung).
static GONE: std::sync::LazyLock<Mutex<HashMap<String, (std::time::Instant, String)>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Meldet yt-dlp etwas Dauerhaftes (gelöscht, privat, gesperrt, Altersfreigabe, nur für
/// Mitglieder)? Vorübergehendes wie die Bot-Prüfung oder Netzfehler zählt nicht.
pub fn is_permanent(msg: &str) -> bool {
    let m = msg.to_lowercase();
    if m.contains("not a bot") || m.contains("rate-limit") || m.contains("timed out") {
        return false;
    }
    [
        "video unavailable",
        "this video is not available",
        "this video is unavailable",
        "private video",
        "has been removed",
        "has been terminated",
        "no longer available",
        "not available in your country",
        "blocked it in your country",
        "who has blocked it",
        "confirm your age",
        "inappropriate for some users",
        "members-only",
        "join this channel",
        "premieres in",
        "copyright",
    ]
    .iter()
    .any(|p| m.contains(p))
}

fn gone(video_id: &str) -> Option<String> {
    let mut g = GONE.lock().unwrap();
    match g.get(video_id) {
        Some((at, msg)) if at.elapsed() < GONE_TTL => Some(msg.clone()),
        Some(_) => {
            g.remove(video_id);
            None
        }
        None => None,
    }
}
/// So lange vor dem Ablauf wird eine Adresse nicht mehr benutzt.
const EXPIRY_MARGIN: i64 = 10 * 60;
/// Ohne `expire` in der Adresse: so lange gilt sie.
const DEFAULT_LIFETIME: i64 = 4 * 60 * 60;
/// So oft wird eine frische Adresse versucht (mit [`RETRY_DELAY`] dazwischen).
const FRESH_TRIES: u32 = 12;
const RETRY_DELAY: Duration = Duration::from_millis(800);
/// Höchstens so viele Adressen im Speicher.
const MAX_CACHED: usize = 500;

/// Aufgelöste Tonspur eines Videos.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub url: String,
    /// Header, die yt-dlp für die Adresse vorgibt (User-Agent u. a.)
    pub headers: Vec<(String, String)>,
    /// "m4a", "webm", …
    pub ext: String,
    pub content_type: &'static str,
    /// Unix-Sekunden
    pub expires_at: i64,
    /// YouTube hat die Adresse schon einmal beantwortet (frische Adressen brauchen ein paar
    /// Sekunden, bis sie gelten)
    pub warm: bool,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Gemerkte Adressen und laufende Auflösungen (damit dasselbe Video nicht zweimal parallel
/// aufgelöst wird, wenn der Browser mehrere Range-Anfragen schickt).
#[derive(Default)]
pub struct StreamCache {
    entries: Mutex<HashMap<String, Resolved>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl StreamCache {
    fn get(&self, id: &str) -> Option<Resolved> {
        let map = self.entries.lock().unwrap();
        map.get(id)
            .filter(|r| r.expires_at - EXPIRY_MARGIN > now())
            .cloned()
    }

    fn put(&self, id: &str, r: Resolved) {
        let mut map = self.entries.lock().unwrap();
        if map.len() >= MAX_CACHED {
            let t = now();
            map.retain(|_, v| v.expires_at - EXPIRY_MARGIN > t);
            if map.len() >= MAX_CACHED {
                map.clear();
            }
        }
        map.insert(id.to_string(), r);
    }

    fn mark_warm(&self, id: &str) {
        if let Some(r) = self.entries.lock().unwrap().get_mut(id) {
            r.warm = true;
        }
    }

    pub fn forget(&self, id: &str) {
        self.entries.lock().unwrap().remove(id);
    }

    fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap();
        if locks.len() > MAX_CACHED {
            locks.retain(|_, l| Arc::strong_count(l) > 1);
        }
        locks.entry(id.to_string()).or_default().clone()
    }
}

/// Content-Type zur Endung, die yt-dlp meldet.
fn content_type(ext: &str, acodec: &str) -> &'static str {
    match ext {
        "m4a" | "mp4" => "audio/mp4",
        "webm" => "audio/webm",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        _ if acodec.starts_with("mp4a") => "audio/mp4",
        _ if acodec == "opus" => "audio/webm",
        _ => "application/octet-stream",
    }
}

/// `expire=` aus der googlevideo-Adresse.
fn expire_param(url: &str) -> Option<i64> {
    let query = url.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("expire="))
        .and_then(|v| v.parse().ok())
}

/// Die JSON-Ausgabe von `yt-dlp -j` auswerten (Felder des gewählten Formats stehen oben).
pub fn parse_info(v: &Value) -> Option<Resolved> {
    let url = v.get("url").and_then(Value::as_str)?.to_string();
    if !url.starts_with("https://") {
        return None;
    }
    let ext = v
        .get("ext")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let acodec = v.get("acodec").and_then(Value::as_str).unwrap_or_default();
    let headers = v
        .get("http_headers")
        .and_then(Value::as_object)
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                // Diese setzt nrzMusic je Anfrage selbst
                .filter(|(k, _)| !k.eq_ignore_ascii_case("range"))
                .collect()
        })
        .unwrap_or_default();
    let expires_at = expire_param(&url).unwrap_or_else(|| now() + DEFAULT_LIFETIME);
    Some(Resolved {
        content_type: content_type(&ext, acodec),
        url,
        headers,
        ext,
        expires_at,
        warm: false,
    })
}

impl Downloads {
    /// Ist yt-dlp für die Wiedergabe da? Sonst 503 (wartet höchstens auf die Einrichtung).
    pub async fn ensure_streamable(&self) -> AppResult<()> {
        self.wait_ready().await;
        if self.available() {
            Ok(())
        } else {
            Err(AppError::Unavailable(
                "yt-dlp wurde nicht gefunden – ohne yt-dlp lassen sich YouTube-Titel nicht abspielen."
                    .into(),
            ))
        }
    }

    /// Adresse der Tonspur (aus dem Speicher oder neu von yt-dlp).
    pub async fn resolve_stream(&self, video_id: &str) -> AppResult<Resolved> {
        if !ytdlp::valid_video_id(video_id) {
            return Err(AppError::bad("Ungültige Video-ID."));
        }
        let cache = &self.inner.streams;
        if let Some(r) = cache.get(video_id) {
            return Ok(r);
        }
        if let Some(msg) = gone(video_id) {
            return Err(AppError::Gone(msg));
        }
        let lock = cache.lock_for(video_id);
        let _guard = lock.lock().await;
        if let Some(r) = cache.get(video_id) {
            return Ok(r);
        }
        if let Some(msg) = gone(video_id) {
            return Err(AppError::Gone(msg));
        }
        self.ensure_streamable().await?;
        let r = self.run_resolve(video_id).await?;
        cache.put(video_id, r.clone());
        Ok(r)
    }

    /// Gemerkte Adresse verwerfen (YouTube hat sie abgelehnt).
    pub fn forget_stream(&self, video_id: &str) {
        self.inner.streams.forget(video_id);
    }

    /// Die Tonspur bei YouTube anfragen (mit `range`, sonst ab Byte 0 – ohne Range drosselt
    /// YouTube). Frische Adressen antworten ein paar Sekunden mit 403: so lange wird
    /// wiederholt; bleibt es dabei, wird die Adresse einmal neu aufgelöst.
    pub async fn open_stream(
        &self,
        video_id: &str,
        range: Option<&str>,
    ) -> AppResult<(Resolved, reqwest::Response)> {
        for round in 0..2 {
            let r = self.resolve_stream(video_id).await?;
            let tries = if r.warm { 3 } else { FRESH_TRIES };
            for attempt in 0..tries {
                if attempt > 0 {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
                let mut req = self.inner.stream_http.get(&r.url);
                for (k, v) in &r.headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                match req.header("Range", range.unwrap_or("bytes=0-")).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        self.inner.streams.mark_warm(video_id);
                        return Ok((r, resp));
                    }
                    Ok(resp) if resp.status().as_u16() == 416 => {
                        return Err(AppError::bad(
                            "Der angefragte Bereich liegt ausserhalb der Datei.",
                        ));
                    }
                    Ok(resp) => {
                        tracing::debug!(video = video_id, status = %resp.status(), attempt, "Tonspur noch abgelehnt")
                    }
                    Err(e) => tracing::debug!(video = video_id, "Tonspur nicht erreichbar: {e}"),
                }
            }
            if round == 0 {
                tracing::info!(video = video_id, "Tonspur abgelehnt – neu auflösen");
                self.forget_stream(video_id);
            }
        }
        Err(AppError::Unavailable(
            "YouTube liefert die Tonspur gerade nicht.".into(),
        ))
    }

    async fn run_resolve(&self, video_id: &str) -> AppResult<Resolved> {
        let tools = self.inner.tools.read().unwrap().clone();
        let Some(program) = tools.ytdlp.clone() else {
            return Err(AppError::Unavailable("yt-dlp wurde nicht gefunden.".into()));
        };
        let mut args: Vec<std::ffi::OsString> = [
            "--ignore-config",
            "--no-playlist",
            "--no-warnings",
            "--no-colors",
            // YouTubes Adressen sind an die IP gebunden: auflösen und abrufen über IPv4
            "--force-ipv4",
            "-j",
            "-f",
            // AAC zuerst: spielt überall (auch Safari/iOS); sonst die beste Tonspur
            "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio/best",
        ]
        .iter()
        .map(Into::into)
        .collect();
        if let Some(js) = tools.js.as_ref().and_then(|j| j.ytdlp_arg()) {
            args.push("--js-runtimes".into());
            args.push(js.into());
        }
        if self.inner.cookies.is_file() {
            // yt-dlp schreibt die Cookie-Datei zurück – eine eigene Kopie nehmen
            let copy = self
                .inner
                .tools_dir
                .join(format!("stream-cookies-{video_id}.txt"));
            if tokio::fs::copy(&self.inner.cookies, &copy).await.is_ok() {
                args.push("--cookies".into());
                args.push(copy.into_os_string());
            }
        }
        args.push("--cache-dir".into());
        args.push(self.inner.tools_dir.join("cache").into_os_string());
        args.push("--".into());
        args.push(ytdlp::watch_url(video_id).into());

        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let out = tokio::time::timeout(RESOLVE_TIMEOUT, cmd.output()).await;
        let _ = tokio::fs::remove_file(
            self.inner
                .tools_dir
                .join(format!("stream-cookies-{video_id}.txt")),
        )
        .await;
        let out = match out {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                return Err(AppError::Unavailable(format!(
                    "yt-dlp liess sich nicht starten: {e}"
                )))
            }
            Err(_) => {
                return Err(AppError::Unavailable(
                    "yt-dlp hat nicht rechtzeitig geantwortet.".into(),
                ))
            }
        };
        if !out.status.success() {
            let mut cap = ytdlp::ErrorCapture::default();
            for line in String::from_utf8_lossy(&out.stderr).lines() {
                cap.push(line);
            }
            let msg = cap
                .message()
                .unwrap_or_else(|| "yt-dlp ist fehlgeschlagen.".into());
            tracing::info!(video = video_id, "Wiedergabe: {msg}");
            if is_permanent(&msg) {
                let mut g = GONE.lock().unwrap();
                if g.len() > 5000 {
                    g.retain(|_, (at, _)| at.elapsed() < GONE_TTL);
                }
                g.insert(video_id.to_string(), (std::time::Instant::now(), msg.clone()));
                return Err(AppError::Gone(msg));
            }
            return Err(AppError::Unavailable(msg));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let line = stdout.lines().find(|l| l.trim_start().starts_with('{'));
        line.and_then(|l| serde_json::from_str::<Value>(l).ok())
            .as_ref()
            .and_then(parse_info)
            .ok_or_else(|| AppError::Unavailable("yt-dlp lieferte keine Tonspur.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permanent_errors() {
        assert!(is_permanent("Video unavailable. This video has been removed by the uploader"));
        assert!(is_permanent("Private video. Sign in if you've been granted access"));
        assert!(is_permanent("Sign in to confirm your age. This video may be inappropriate for some users."));
        assert!(!is_permanent("Sign in to confirm you’re not a bot"));
        assert!(!is_permanent("Unable to download webpage: timed out"));
    }

    #[test]
    fn info_from_ytdlp_json() {
        let v = json!({
            "id": "dQw4w9WgXcQ",
            "url": "https://rr1---sn-x.googlevideo.com/videoplayback?expire=1900000000&itag=140&x=y",
            "ext": "m4a",
            "acodec": "mp4a.40.2",
            "http_headers": {"User-Agent": "Mozilla/5.0", "Accept": "*/*", "Range": "x"}
        });
        let r = parse_info(&v).unwrap();
        assert_eq!(r.ext, "m4a");
        assert_eq!(r.content_type, "audio/mp4");
        assert_eq!(r.expires_at, 1_900_000_000);
        assert!(r.headers.iter().any(|(k, _)| k == "User-Agent"));
        assert!(!r.headers.iter().any(|(k, _)| k == "Range"));

        let webm = json!({"url": "https://x.googlevideo.com/v", "ext": "webm", "acodec": "opus"});
        let r = parse_info(&webm).unwrap();
        assert_eq!(r.content_type, "audio/webm");
        assert!(r.expires_at > now());

        assert!(parse_info(&json!({"url": "file:///etc/passwd"})).is_none());
        assert!(parse_info(&json!({"ext": "m4a"})).is_none());
    }

    #[test]
    fn cache_respects_expiry() {
        let c = StreamCache::default();
        let mk = |exp| Resolved {
            url: "https://x".into(),
            headers: vec![],
            ext: "m4a".into(),
            content_type: "audio/mp4",
            expires_at: exp,
            warm: false,
        };
        c.put("a", mk(now() + 3600));
        c.put("b", mk(now() + 60));
        assert!(c.get("a").is_some());
        assert!(c.get("b").is_none());
        c.forget("a");
        assert!(c.get("a").is_none());
    }
}
