//! Tonspuren von YouTube-Titeln als ganze Datei auf dem Server (`<data_dir>/cache/audio/`).
//!
//! YouTubes Tonspuren sind fragmentierte MP4 (DASH). WebKit (Safari, macOS-App, iPhone) liest
//! deren Länge falsch – oft doppelt so lang – und spielt nach dem echten Ende stumm weiter: das
//! letzte Stück eines Titels blieb still, der nächste kam erst viel später. Darum verpackt der
//! Server jede Tonspur nach dem Laden mit ffmpeg ohne Neukodieren in eine normale M4A
//! (`-c copy -movflags +faststart`). Solange das nicht fertig ist, bekommt der Player nichts
//! von der Rohdatei (ausser ohne ffmpeg).
//!
//! Früher reichte der Server die Bytes von YouTube live an den Player durch. YouTube bricht
//! lange, offene Verbindungen aber gern ab (Drosselung) – Safari/WebKit (macOS-App, iPhone) hört
//! dann mitten im Titel auf, und jeder Sprung kostet eine neue Anfrage bei YouTube. Jetzt lädt der
//! Server die Tonspur so, wie YouTubes eigener Player: in Stücken (`Range` je 2 MiB), legt sie
//! als Datei ab und liefert sie wie einen Titel aus dem Musikordner (Länge bekannt, Spulen ohne
//! YouTube). Ein Titel (3–8 MB) liegt meist in ein bis zwei Sekunden vor.
//!
//! Wer einen Titel abspielt, der noch lädt, bekommt ihn schon während des Ladens
//! ([`Source::Growing`]): Länge und Bereich stimmen von Anfang an, Bytes, die noch fehlen, wartet
//! die Antwort ab. Bricht ein Stück bei YouTube ab, wird es wiederholt (auch mit neu
//! aufgelöster Adresse) – die Antwort an den Player läuft weiter.
//!
//! Die Oberfläche lässt die nächsten Titel vorab holen (`POST /yt/stream/<id>/prepare`) – der
//! Wechsel ist dann sofort da. Höchstens [`MAX_PARALLEL`] Titel laden gleichzeitig; der Ordner
//! bleibt unter [`MAX_BYTES`] (die am längsten nicht gespielten fliegen zuerst).

use super::Downloads;
use crate::error::{AppError, AppResult};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::io::AsyncWriteExt;
use tokio::sync::{watch, Semaphore};

/// Grösse des ersten Stücks (schnell bereit) und der weiteren.
const FIRST_CHUNK: u64 = 1024 * 1024;
const CHUNK: u64 = 2 * 1024 * 1024;
/// Gleichzeitig ladende Titel.
const MAX_PARALLEL: usize = 3;
/// Obergrenze des Zwischenspeichers.
const MAX_BYTES: u64 = 1024 * 1024 * 1024;
/// Höchstens so gross darf eine Tonspur sein (Schutz vor endlosen Livestreams).
const MAX_FILE: u64 = 300 * 1024 * 1024;

/// Fertig geladene Tonspur.
#[derive(Clone, Debug)]
pub struct CachedAudio {
    pub path: PathBuf,
    pub content_type: &'static str,
}

/// Stand eines laufenden Ladevorgangs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fill {
    /// So viele Bytes stehen (von vorn) in der `.part`-Datei.
    pub have: u64,
    /// Gesamtgrösse (sobald das erste Stück da ist).
    pub total: Option<u64>,
    pub failed: bool,
}

/// Ein laufender Ladevorgang.
pub struct Inflight {
    pub part: PathBuf,
    fill: watch::Sender<Fill>,
    content_type: std::sync::OnceLock<&'static str>,
    result: watch::Sender<Option<Result<CachedAudio, String>>>,
}

impl Inflight {
    pub fn content_type(&self) -> &'static str {
        self.content_type.get().copied().unwrap_or("application/octet-stream")
    }

    pub fn fill(&self) -> watch::Receiver<Fill> {
        self.fill.subscribe()
    }
}

/// Woher die Wiedergabe kommt.
pub enum Source {
    /// Ganz geladen.
    Done(CachedAudio),
    /// Lädt noch – Gesamtgrösse ist bekannt.
    Growing(Arc<Inflight>, u64),
}

pub struct AudioCache {
    dir: PathBuf,
    inflight: Mutex<HashMap<String, Arc<Inflight>>>,
    permits: Arc<Semaphore>,
}

impl AudioCache {
    pub fn new(dir: PathBuf) -> Self {
        // Zwischenspeicher von 1.4.0 (Rohdateien, die WebKit falsch liest) weg
        if let Some(old) = dir.parent().map(|p| p.join("stream")) {
            let _ = std::fs::remove_dir_all(old);
        }
        // Reste abgebrochener Downloads wegräumen
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if e.path().extension().is_some_and(|x| x == "part" || x == "remux") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        AudioCache {
            dir,
            inflight: Mutex::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(MAX_PARALLEL)),
        }
    }

    /// Schon fertig geladen? (Merkt den Zugriff für das Aufräumen.)
    fn lookup(&self, video_id: &str) -> Option<CachedAudio> {
        for (ext, content_type) in [("m4a", "audio/mp4"), ("webm", "audio/webm"), ("mp3", "audio/mpeg"), ("ogg", "audio/ogg"), ("bin", "application/octet-stream")] {
            let path = self.dir.join(format!("{video_id}.{ext}"));
            if path.is_file() {
                if let Ok(f) = std::fs::File::options().append(true).open(&path) {
                    let _ = f.set_modified(SystemTime::now());
                }
                return Some(CachedAudio { path, content_type });
            }
        }
        None
    }

    /// Ordner unter [`MAX_BYTES`] halten: die am längsten nicht benutzten Dateien zuerst.
    fn evict(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x != "part" && x != "remux"))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        files.sort_by_key(|f| f.0);
        for (_, len, path) in files {
            if total <= MAX_BYTES {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}

/// Endung der Datei zum Content-Type.
fn ext_for(content_type: &str) -> &'static str {
    match content_type {
        "audio/mp4" => "m4a",
        "audio/webm" => "webm",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        _ => "bin",
    }
}

/// `bytes 0-1048575/3456789` → Gesamtgrösse.
fn total_from_content_range(v: &str) -> Option<u64> {
    v.rsplit('/').next()?.trim().parse().ok()
}

impl Downloads {
    /// Die Tonspur als Datei – sofort, wenn sie schon da ist, sonst nach dem Laden (höchstens
    /// `wait`). Lädt im Hintergrund weiter, auch wenn niemand mehr wartet.
    pub async fn cached_audio(&self, video_id: &str, wait: Duration) -> AppResult<CachedAudio> {
        if !super::ytdlp::valid_video_id(video_id) {
            return Err(AppError::bad("Ungültige Video-ID."));
        }
        if let Some(c) = self.inner.audio_cache.lookup(video_id) {
            return Ok(c);
        }
        let mut rx = self.start_fetch(video_id).result.subscribe();
        let done = tokio::time::timeout(wait, async {
            loop {
                if let Some(r) = rx.borrow_and_update().clone() {
                    return r;
                }
                if rx.changed().await.is_err() {
                    return Err("Laden abgebrochen".to_string());
                }
            }
        })
        .await;
        match done {
            Ok(Ok(c)) => Ok(c),
            Ok(Err(e)) => Err(AppError::Unavailable(e)),
            Err(_) => Err(AppError::Unavailable("Die Tonspur lädt noch.".into())),
        }
    }

    /// Für die Wiedergabe: fertige Datei oder – sobald die Grösse bekannt ist – der laufende
    /// Ladevorgang (höchstens `wait` warten, das Auflösen mit yt-dlp dauert ein paar Sekunden).
    pub async fn audio_source(&self, video_id: &str, wait: Duration) -> AppResult<Source> {
        if !super::ytdlp::valid_video_id(video_id) {
            return Err(AppError::bad("Ungültige Video-ID."));
        }
        if let Some(c) = self.inner.audio_cache.lookup(video_id) {
            return Ok(Source::Done(c));
        }
        let remux = self.inner.tools.read().unwrap().ffmpeg.is_some();
        let inflight = self.start_fetch(video_id);
        let mut fill = inflight.fill();
        let mut result = inflight.result.subscribe();
        let r = tokio::time::timeout(wait, async {
            loop {
                if let Some(r) = result.borrow_and_update().clone() {
                    return r.map(Source::Done);
                }
                let f = *fill.borrow_and_update();
                if f.failed {
                    return Err("Laden fehlgeschlagen".to_string());
                }
                // Während des Ladens nur ohne ffmpeg (sonst kommt gleich die umverpackte Datei)
                if let Some(total) = f.total {
                    if f.have > 0 && !remux {
                        return Ok(Source::Growing(inflight.clone(), total));
                    }
                }
                tokio::select! {
                    r = fill.changed() => if r.is_err() { tokio::time::sleep(Duration::from_millis(20)).await },
                    _ = result.changed() => {}
                }
            }
        })
        .await;
        match r {
            Ok(Ok(s)) => Ok(s),
            Ok(Err(e)) => Err(AppError::Unavailable(e)),
            Err(_) => Err(AppError::Unavailable("Die Tonspur lädt noch.".into())),
        }
    }

    /// Laden beginnen (oder den laufenden Ladevorgang zurückgeben).
    fn start_fetch(&self, video_id: &str) -> Arc<Inflight> {
        let cache = &self.inner.audio_cache;
        let mut map = cache.inflight.lock().unwrap();
        if let Some(i) = map.get(video_id) {
            return i.clone();
        }
        let inflight = Arc::new(Inflight {
            part: cache.dir.join(format!("{video_id}.part")),
            fill: watch::channel(Fill::default()).0,
            content_type: std::sync::OnceLock::new(),
            result: watch::channel(None).0,
        });
        map.insert(video_id.to_string(), inflight.clone());
        drop(map);
        let this = self.clone();
        let id = video_id.to_string();
        let job = inflight.clone();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let result = {
                let _permit = this.inner.audio_cache.permits.clone().acquire_owned().await;
                this.fetch_audio(&id, &job).await
            };
            match &result {
                Ok(c) => tracing::info!(video = %id, ms = started.elapsed().as_millis() as u64, "Tonspur geladen: {}", c.path.display()),
                Err(e) => {
                    tracing::info!(video = %id, "Tonspur nicht geladen: {e}");
                    job.fill.send_modify(|f| f.failed = true);
                }
            }
            let _ = job.result.send(Some(result.map_err(|e| e.to_string())));
            this.inner.audio_cache.inflight.lock().unwrap().remove(&id);
            this.inner.audio_cache.evict();
        });
        inflight
    }

    /// Tonspur in Stücken von YouTube holen und als `<id>.<ext>` ablegen.
    async fn fetch_audio(&self, video_id: &str, job: &Inflight) -> AppResult<CachedAudio> {
        let dir = self.inner.audio_cache.dir.clone();
        tokio::fs::create_dir_all(&dir).await?;
        let part = job.part.clone();
        let result = self.fetch_into(video_id, job).await;
        match result {
            Ok(content_type) => {
                let path = dir.join(format!("{video_id}.{}", ext_for(content_type)));
                if content_type == "audio/mp4" {
                    self.remux(&part, &path).await;
                }
                if !path.is_file() {
                    tokio::fs::rename(&part, &path).await?;
                } else {
                    let _ = tokio::fs::remove_file(&part).await;
                }
                Ok(CachedAudio { path, content_type })
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                Err(e)
            }
        }
    }

    /// Fragmentierte MP4 (DASH) ohne Neukodieren in eine normale M4A umpacken. Schlägt das fehl
    /// (oder fehlt ffmpeg), bleibt die Rohdatei.
    async fn remux(&self, part: &std::path::Path, target: &std::path::Path) {
        let Some(ffmpeg) = self.inner.tools.read().unwrap().ffmpeg.clone() else { return };
        let tmp = target.with_extension("remux");
        let mut cmd = tokio::process::Command::new(&ffmpeg);
        cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(part)
            .args(["-map", "0:a:0", "-c", "copy", "-movflags", "+faststart", "-f", "mp4"])
            .arg(&tmp)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        match tokio::time::timeout(Duration::from_secs(60), cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => {
                if let Err(e) = tokio::fs::rename(&tmp, target).await {
                    tracing::warn!("Umpacken: {e}");
                }
            }
            Ok(Ok(out)) => tracing::warn!("Umpacken fehlgeschlagen: {}", String::from_utf8_lossy(&out.stderr).trim()),
            Ok(Err(e)) => tracing::warn!("ffmpeg startet nicht: {e}"),
            Err(_) => tracing::warn!("Umpacken dauert zu lange"),
        }
        let _ = tokio::fs::remove_file(&tmp).await;
    }

    async fn fetch_into(&self, video_id: &str, job: &Inflight) -> AppResult<&'static str> {
        let mut file = tokio::fs::File::create(&job.part).await?;
        // Erstes Stück: wartet bei frischen Adressen auf YouTube (403 die ersten Sekunden)
        let (resolved, resp) = self.open_stream(video_id, Some(&format!("bytes=0-{}", FIRST_CHUNK - 1))).await?;
        let content_type = resolved.content_type;
        let _ = job.content_type.set(content_type);
        let total = resp
            .headers()
            .get("Content-Range")
            .and_then(|v| v.to_str().ok())
            .and_then(total_from_content_range);
        let mut have = write_body(&mut file, resp).await?;
        file.flush().await?;
        job.fill.send_modify(|f| {
            f.have = have;
            f.total = Some(total.unwrap_or(have));
        });
        if let Some(total) = total {
            if total > MAX_FILE {
                return Err(AppError::Unavailable("Die Tonspur ist zu gross (Livestream?).".into()));
            }
            while have < total {
                let end = (have + CHUNK).min(total) - 1;
                let range = format!("bytes={have}-{end}");
                let mut last_err = None;
                let mut ok = false;
                for attempt in 0..4 {
                    if attempt > 0 {
                        tokio::time::sleep(Duration::from_millis(400 * attempt)).await;
                    }
                    match self.open_stream(video_id, Some(&range)).await {
                        Ok((_, resp)) => match write_chunk(&mut file, resp, end + 1 - have).await {
                            Ok(n) => {
                                have += n;
                                file.flush().await?;
                                job.fill.send_modify(|f| f.have = have);
                                ok = true;
                                break;
                            }
                            Err(e) => last_err = Some(e),
                        },
                        Err(e) => last_err = Some(e),
                    }
                }
                if !ok {
                    return Err(last_err.unwrap_or_else(|| AppError::Unavailable("YouTube liefert die Tonspur gerade nicht.".into())));
                }
            }
            if have != total {
                return Err(AppError::Unavailable(format!("Tonspur unvollständig ({have} von {total} Bytes)")));
            }
        }
        file.flush().await?;
        file.sync_all().await.ok();
        Ok(content_type)
    }
}

/// Ganze Antwort in die Datei schreiben; Anzahl Bytes.
async fn write_body(file: &mut tokio::fs::File, mut resp: reqwest::Response) -> AppResult<u64> {
    let mut n: u64 = 0;
    while let Some(chunk) = resp.chunk().await.map_err(|e| AppError::Unavailable(format!("Verbindung zu YouTube abgebrochen: {e}")))? {
        file.write_all(&chunk).await?;
        n += chunk.len() as u64;
        if n > MAX_FILE {
            return Err(AppError::Unavailable("Die Tonspur ist zu gross (Livestream?).".into()));
        }
    }
    Ok(n)
}

/// Ein Stück schreiben – nur, wenn es vollständig ankam (sonst wird es wiederholt).
async fn write_chunk(file: &mut tokio::fs::File, mut resp: reqwest::Response, expected: u64) -> AppResult<u64> {
    let mut buf: Vec<u8> = Vec::with_capacity(expected as usize);
    while let Some(chunk) = resp.chunk().await.map_err(|e| AppError::Unavailable(format!("Verbindung zu YouTube abgebrochen: {e}")))? {
        buf.extend_from_slice(&chunk);
        if buf.len() as u64 > expected {
            break;
        }
    }
    if buf.len() as u64 != expected {
        return Err(AppError::Unavailable(format!("Stück unvollständig ({} von {expected} Bytes)", buf.len())));
    }
    file.write_all(&buf).await?;
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_total() {
        assert_eq!(total_from_content_range("bytes 0-1048575/3456789"), Some(3_456_789));
        assert_eq!(total_from_content_range("bytes */100"), Some(100));
        assert_eq!(total_from_content_range("bytes 0-1/*"), None);
    }

    #[test]
    fn extensions() {
        assert_eq!(ext_for("audio/mp4"), "m4a");
        assert_eq!(ext_for("audio/webm"), "webm");
        assert_eq!(ext_for("x"), "bin");
    }

    #[test]
    fn eviction_keeps_newest_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.part"), b"x").unwrap();
        let cache = AudioCache::new(dir.path().to_path_buf());
        assert!(!dir.path().join("a.part").exists(), "Reste werden beim Start gelöscht");
        std::fs::write(dir.path().join("abcdefghijk.m4a"), b"data").unwrap();
        let hit = cache.lookup("abcdefghijk").unwrap();
        assert_eq!(hit.content_type, "audio/mp4");
        cache.evict();
        assert!(hit.path.exists(), "unter der Grenze bleibt alles");
    }
}
