//! Downloads mit yt-dlp: eine Warteschlange in der Datenbank (`download_jobs`), N Arbeiter,
//! je Auftrag ein Kindprozess (Argumentliste, keine Shell) in einem eigenen Arbeitsordner unter
//! `data/downloads/`. Danach schreibt nrzMusic Tags und Cover selbst, legt die Datei unter
//! `<Musik>/YouTube/<Künstler>/<Album oder Singles>/<Titel>.<ext>` ab (nie überschreiben) und
//! liest sie gleich ein – wie einen Upload.
//!
//! [`Downloads`] ist der gemeinsame Zustand (Rocket-State, billig zu klonen).

pub mod stream;
pub mod tagging;
pub mod tool;
pub mod ytdlp;

use crate::config::Config;
use crate::db::downloads as jobs;
use crate::db::library as store;
use crate::db::settings::SettingsStore;
use crate::error::{AppError, AppResult};
use crate::library::{audio_format, Events, Library};
use crate::routes::api::upload::{hidden_temp_name, prepare_dir, publish_temp};
use nrzmusic_types::{
    DownloadJob, DownloadStatus, DownloadedTrack, ServerEvent, YtDlpInfo, YtTrack,
};
use rocket::Shutdown;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{mpsc, oneshot, watch, Notify};

/// Höchstens so oft ein Fortschritts-Ereignis je Auftrag (~2 pro Sekunde).
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
/// Wie oft yt-dlp sich selbst aktualisiert (wenn eingeschaltet).
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// So lange warten Anfragen höchstens darauf, dass yt-dlp eingerichtet ist.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// So viele Titel nimmt ein Download-Wunsch höchstens an.
pub const MAX_REQUEST_TRACKS: usize = 5000;

/// Ergebnis eines Auftrags.
enum Outcome {
    Done(i64),
    Failed(String),
    Canceled,
}

#[derive(Clone)]
pub struct Downloads {
    inner: Arc<Inner>,
}

struct Inner {
    pool: SqlitePool,
    library: Library,
    events: Events,
    settings: SettingsStore,
    http: reqwest::Client,
    configured_ytdlp: String,
    configured_ffmpeg: String,
    tools_dir: PathBuf,
    work_dir: PathBuf,
    cookies: PathBuf,
    workers: usize,
    /// Gefundene Programme (nach der Einrichtung)
    tools: RwLock<tool::Tools>,
    /// Zeitpunkt und Meldung der letzten Selbst-Aktualisierung
    last_update: Mutex<(Option<i64>, Option<String>)>,
    /// `true`, sobald die Einrichtung durch ist
    ready: watch::Sender<bool>,
    /// Weckt wartende Arbeiter (neuer Auftrag, Downloads eingeschaltet)
    wake: Notify,
    /// Laufende Aufträge → Abbruch-Signal
    running: Mutex<HashMap<i64, oneshot::Sender<()>>>,
    update_lock: tokio::sync::Mutex<()>,
    /// Aufgelöste Tonspuren für die direkte Wiedergabe
    streams: stream::StreamCache,
    /// Client fürs Durchreichen der Tonspur (ohne Gesamt-Zeitlimit)
    stream_http: reqwest::Client,
}

impl Downloads {
    pub fn new(
        pool: SqlitePool,
        library: Library,
        events: Events,
        settings: SettingsStore,
        config: &Config,
    ) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("nrzMusic/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Downloads {
            inner: Arc::new(Inner {
                pool,
                library,
                events,
                settings,
                http,
                configured_ytdlp: config.ytdlp.clone(),
                configured_ffmpeg: config.ffmpeg.clone(),
                tools_dir: config.tools_dir(),
                work_dir: config.downloads_dir(),
                cookies: config.cookies_path(),
                workers: config.download_workers.clamp(1, 16),
                tools: RwLock::new(tool::Tools::default()),
                last_update: Mutex::new((None, None)),
                ready: watch::channel(false).0,
                wake: Notify::new(),
                running: Mutex::new(HashMap::new()),
                update_lock: tokio::sync::Mutex::new(()),
                streams: stream::StreamCache::default(),
                stream_http: reqwest::Client::builder()
                    // wie yt-dlp (--force-ipv4): die Adresse gilt nur für diese IP
                    .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
                    .connect_timeout(Duration::from_secs(10))
                    .read_timeout(Duration::from_secs(30))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new()),
            }),
        }
    }

    fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    fn emit(&self, job: &DownloadJob) {
        self.inner
            .events
            .send(ServerEvent::DownloadUpdate(Box::new(job.clone())));
    }

    /// Einrichten (yt-dlp kopieren, Programme suchen), Arbeiter starten und – mit
    /// `background` – die tägliche Selbst-Aktualisierung. Alles endet mit dem Server.
    pub fn start(&self, shutdown: Shutdown, background: bool) {
        let this = self.clone();
        tokio::spawn(async move {
            this.setup().await;
            // Ohne Downloads (NRZMUSIC_DOWNLOADS=false) bleibt yt-dlp für die Wiedergabe da
            if !this.inner.settings.downloads_allowed() {
                return;
            }
            for n in 0..this.inner.workers {
                tokio::spawn(this.clone().worker(n, shutdown.clone()));
            }
            if background {
                tokio::spawn(this.clone().auto_update(shutdown));
            }
        });
    }

    async fn setup(&self) {
        // Reste abgebrochener Läufe weg; was "running" stehen blieb, wartet wieder
        let _ = tokio::fs::remove_dir_all(&self.inner.work_dir).await;
        if let Err(e) = tokio::fs::create_dir_all(&self.inner.work_dir).await {
            tracing::warn!(
                "Arbeitsordner {} nicht anlegbar: {e}",
                self.inner.work_dir.display()
            );
        }
        match jobs::requeue_running(self.pool()).await {
            Ok(n) if n > 0 => tracing::info!(n, "Unterbrochene Downloads wieder eingereiht"),
            Err(e) => tracing::warn!("Unterbrochene Downloads: {e}"),
            _ => {}
        }
        let tools = tool::setup(
            &self.inner.configured_ytdlp,
            &self.inner.configured_ffmpeg,
            &self.inner.tools_dir,
        )
        .await;
        match (&tools.ytdlp, &tools.version) {
            (Some(p), Some(v)) => tracing::info!(
                version = %v,
                ffmpeg = tools.ffmpeg.is_some(),
                js = %tools.js.as_ref().map(|j| j.label()).unwrap_or_else(|| "–".into()),
                "yt-dlp: {}",
                p.display()
            ),
            _ => tracing::warn!(
                "{}",
                tools.message.as_deref().unwrap_or("yt-dlp nicht gefunden")
            ),
        }
        *self.inner.tools.write().unwrap() = tools;
        self.inner.ready.send_replace(true);
    }

    /// Wartet, bis die Einrichtung durch ist (höchstens [`READY_TIMEOUT`]).
    pub async fn wait_ready(&self) {
        let mut rx = self.inner.ready.subscribe();
        let _ = tokio::time::timeout(READY_TIMEOUT, rx.wait_for(|r| *r)).await;
    }

    /// Ist yt-dlp gefunden und lauffähig?
    pub fn available(&self) -> bool {
        let t = self.inner.tools.read().unwrap();
        t.ytdlp.is_some() && t.version.is_some()
    }

    /// `GET /downloads/tool`
    pub fn info(&self) -> YtDlpInfo {
        let t = self.inner.tools.read().unwrap().clone();
        let (last_update, update_message) = self.inner.last_update.lock().unwrap().clone();
        let message = if !self.inner.settings.downloads_allowed() {
            Some("Downloads sind ausgeschaltet (NRZMUSIC_DOWNLOADS=false).".into())
        } else {
            update_message.or(t.message.clone())
        };
        YtDlpInfo {
            available: t.ytdlp.is_some() && t.version.is_some(),
            version: t.version,
            ffmpeg: t.ffmpeg.is_some(),
            js_runtime: t.js.as_ref().map(|j| j.label()),
            cookies: self.inner.cookies.is_file(),
            last_update,
            message,
        }
    }

    /// Sind Downloads eingeschaltet (Umgebung und Einstellung)? Sonst 503.
    pub fn ensure_enabled(&self) -> AppResult<()> {
        if self.inner.settings.downloads_enabled() {
            Ok(())
        } else {
            Err(AppError::Unavailable(
                "Downloads sind ausgeschaltet.".into(),
            ))
        }
    }

    /// Eingeschaltet und yt-dlp vorhanden? Sonst 503.
    pub async fn ensure_usable(&self) -> AppResult<()> {
        self.ensure_enabled()?;
        self.wait_ready().await;
        if self.available() {
            Ok(())
        } else {
            Err(AppError::Unavailable(
                "yt-dlp wurde nicht gefunden – installieren oder NRZMUSIC_YTDLP setzen.".into(),
            ))
        }
    }

    /// Einstellungen geändert: Arbeiter wecken (Downloads evtl. wieder an).
    pub fn settings_changed(&self) {
        self.inner.wake.notify_waiters();
    }

    /// Liegt die Datei eines lokalen Titels noch im Musikordner?
    async fn track_file_exists(&self, track_id: Option<i64>) -> AppResult<bool> {
        let Some(id) = track_id else {
            return Ok(false);
        };
        Ok(match store::get(self.pool(), id).await? {
            Some(t) => self.inner.library.abs_path(&t.path).is_file(),
            None => false,
        })
    }

    /// Titel in die Warteschlange stellen. Laufende, wartende und schon geladene Titel bekommen
    /// ihren bestehenden Auftrag zurück; fehlgeschlagene, abgebrochene und solche, deren Datei
    /// gelöscht wurde, werden neu eingereiht. Ungültige Video-IDs → 400 (nichts wird angelegt).
    pub async fn enqueue(&self, tracks: &[YtTrack]) -> AppResult<Vec<DownloadJob>> {
        if tracks.len() > MAX_REQUEST_TRACKS {
            return Err(AppError::bad(format!(
                "Höchstens {MAX_REQUEST_TRACKS} Titel auf einmal."
            )));
        }
        if let Some(bad) = tracks.iter().find(|t| !ytdlp::valid_video_id(&t.video_id)) {
            return Err(AppError::bad(format!(
                "Ungültige YouTube-Video-ID: „{}“.",
                bad.video_id.chars().take(40).collect::<String>()
            )));
        }
        let mut out = Vec::with_capacity(tracks.len());
        let mut seen = HashSet::new();
        let mut added = false;
        for t in tracks {
            if !seen.insert(t.video_id.clone()) {
                continue;
            }
            if let Some(existing) = jobs::by_video(self.pool(), &t.video_id).await? {
                let keep = match existing.status {
                    DownloadStatus::Queued | DownloadStatus::Running => true,
                    DownloadStatus::Done => self.track_file_exists(existing.track_id).await?,
                    DownloadStatus::Failed | DownloadStatus::Canceled => false,
                };
                if keep {
                    jobs::unhide(self.pool(), existing.id).await?;
                    out.push(existing);
                    continue;
                }
            }
            let job = jobs::enqueue(self.pool(), t).await?;
            self.emit(&job);
            out.push(job);
            added = true;
        }
        if added {
            self.inner.wake.notify_waiters();
        }
        Ok(out)
    }

    pub async fn list(&self) -> AppResult<Vec<DownloadJob>> {
        jobs::list(self.pool()).await
    }

    /// Auftrag neu versuchen (fehlgeschlagen, abgebrochen oder Datei gelöscht); sonst bleibt er.
    pub async fn retry(&self, id: i64) -> AppResult<DownloadJob> {
        let job = jobs::get(self.pool(), id)
            .await?
            .ok_or(AppError::NotFound)?;
        let again = match job.status {
            DownloadStatus::Failed | DownloadStatus::Canceled => true,
            DownloadStatus::Done => !self.track_file_exists(job.track_id).await?,
            DownloadStatus::Queued | DownloadStatus::Running => false,
        };
        if !again {
            return Ok(job);
        }
        let job = jobs::enqueue(self.pool(), &job.track).await?;
        self.emit(&job);
        self.inner.wake.notify_waiters();
        Ok(job)
    }

    /// Wartenden/laufenden Auftrag abbrechen (laufendes yt-dlp wird beendet) oder einen
    /// fertigen aus der Liste nehmen (die Datei bleibt).
    pub async fn remove(&self, id: i64) -> AppResult<()> {
        let job = jobs::get(self.pool(), id)
            .await?
            .ok_or(AppError::NotFound)?;
        match job.status {
            DownloadStatus::Queued | DownloadStatus::Running => {
                if jobs::cancel(self.pool(), id).await? {
                    if let Some(tx) = self.inner.running.lock().unwrap().remove(&id) {
                        let _ = tx.send(());
                    }
                    if let Some(job) = jobs::get(self.pool(), id).await? {
                        self.emit(&job);
                    }
                }
            }
            _ => {
                jobs::remove_finished(self.pool(), id).await?;
            }
        }
        Ok(())
    }

    pub async fn clear_finished(&self) -> AppResult<()> {
        jobs::clear_finished(self.pool()).await
    }

    /// Welche YouTube-Titel als lokale Datei vorliegen (nur, wo die Datei noch existiert).
    pub async fn map(&self) -> AppResult<Vec<DownloadedTrack>> {
        let rows = jobs::done_with_paths(self.pool()).await?;
        let mut seen = HashSet::new();
        Ok(rows
            .into_iter()
            .filter(|(video, _, path)| {
                self.inner.library.abs_path(path).is_file() && seen.insert(video.clone())
            })
            .map(|(video_id, track_id, _)| DownloadedTrack { video_id, track_id })
            .collect())
    }

    /// `yt-dlp -U` auf der eigenen Kopie, danach Version und Programme neu erkennen.
    pub async fn update_tool(&self) -> YtDlpInfo {
        let _guard = self.inner.update_lock.lock().await;
        let program = self.inner.tools.read().unwrap().ytdlp.clone();
        let message = match &program {
            Some(p) => tool::self_update(p).await,
            None => "yt-dlp wurde nicht gefunden.".into(),
        };
        tracing::info!("yt-dlp -U: {message}");
        *self.inner.last_update.lock().unwrap() = (Some(crate::db::now()), Some(message));
        let tools = tool::setup(
            &self.inner.configured_ytdlp,
            &self.inner.configured_ffmpeg,
            &self.inner.tools_dir,
        )
        .await;
        *self.inner.tools.write().unwrap() = tools;
        self.info()
    }

    /// Beim Start und danach alle 24 Stunden `yt-dlp -U` (wenn eingeschaltet).
    async fn auto_update(self, mut shutdown: Shutdown) {
        loop {
            if self.inner.settings.get().ytdlp_auto_update && self.available() {
                self.update_tool().await;
            }
            tokio::select! {
                _ = tokio::time::sleep(UPDATE_EVERY) => {}
                _ = &mut shutdown => break,
            }
        }
    }

    /// Ein Arbeiter: holt sich wartende Aufträge, solange Downloads eingeschaltet sind.
    async fn worker(self, n: usize, mut shutdown: Shutdown) {
        tracing::debug!(n, "Download-Arbeiter gestartet");
        loop {
            // Erst anmelden, dann nachsehen – kein Wecken geht verloren
            let notified = self.inner.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.inner.settings.downloads_enabled() && self.available() {
                match jobs::claim_next(self.pool()).await {
                    Ok(Some(job)) => {
                        tokio::select! {
                            _ = self.process(job) => {}
                            _ = &mut shutdown => break,
                        }
                        continue;
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("Download-Warteschlange: {e}"),
                }
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                _ = &mut shutdown => break,
            }
        }
    }

    /// Einen Auftrag ausführen und das Ergebnis festhalten.
    async fn process(&self, mut job: DownloadJob) {
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.inner.running.lock().unwrap().insert(job.id, cancel_tx);
        self.emit(&job);
        let dir = self.inner.work_dir.join(format!(
            "job-{}-{}",
            job.id,
            hex::encode(rand::random::<[u8; 4]>())
        ));
        let started = Instant::now();
        let outcome = self.run(&mut job, &dir, cancel_rx).await;
        self.inner.running.lock().unwrap().remove(&job.id);
        let _ = tokio::fs::remove_dir_all(&dir).await;

        let result = match &outcome {
            Outcome::Done(track_id) => jobs::finish_done(self.pool(), job.id, *track_id)
                .await
                .map(|_| ()),
            Outcome::Failed(msg) => jobs::finish_failed(self.pool(), job.id, msg)
                .await
                .map(|_| ()),
            Outcome::Canceled => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!(job = job.id, "Download-Ergebnis nicht speicherbar: {e}");
        }
        match &outcome {
            Outcome::Done(id) => tracing::info!(
                video = %job.track.video_id,
                track = id,
                secs = started.elapsed().as_secs_f32(),
                "Heruntergeladen"
            ),
            Outcome::Failed(msg) => {
                tracing::warn!(video = %job.track.video_id, "Download fehlgeschlagen: {msg}")
            }
            Outcome::Canceled => {
                tracing::info!(video = %job.track.video_id, "Download abgebrochen")
            }
        }
        match jobs::get(self.pool(), job.id).await {
            Ok(Some(j)) => self.emit(&j),
            Ok(None) => {}
            Err(e) => tracing::warn!("Download-Auftrag lesen: {e}"),
        }
    }

    /// Fortschritt setzen; Ereignis und Datenbank höchstens ~2× pro Sekunde (oder `force`).
    async fn report(&self, job: &mut DownloadJob, progress: f32, last: &mut Instant, force: bool) {
        if (progress - job.progress).abs() < 0.005 && !force {
            return;
        }
        job.progress = progress;
        if force || last.elapsed() >= PROGRESS_EVERY {
            *last = Instant::now();
            if let Err(e) = jobs::set_progress(self.pool(), job.id, progress).await {
                tracing::debug!("Fortschritt speichern: {e}");
            }
            self.emit(job);
        }
    }

    /// yt-dlp laufen lassen, taggen, ablegen, einlesen.
    async fn run(
        &self,
        job: &mut DownloadJob,
        dir: &Path,
        mut cancel: oneshot::Receiver<()>,
    ) -> Outcome {
        let tools = self.inner.tools.read().unwrap().clone();
        let Some(program) = tools.ytdlp.clone() else {
            return Outcome::Failed("yt-dlp wurde nicht gefunden.".into());
        };
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            return Outcome::Failed(format!("Arbeitsordner nicht anlegbar: {e}"));
        }
        let format = {
            let f = self.inner.settings.get().download_format;
            if crate::db::settings::DOWNLOAD_FORMATS.contains(&f.as_str()) {
                f
            } else {
                "mp3".to_string()
            }
        };
        // yt-dlp schreibt die Cookie-Datei zurück – jeder Auftrag bekommt seine eigene Kopie
        let cookies = if self.inner.cookies.is_file() {
            let copy = dir.join("cookies.txt");
            tokio::fs::copy(&self.inner.cookies, &copy)
                .await
                .ok()
                .map(|_| copy)
        } else {
            None
        };
        let out_dir = dir.join("out");
        let cache_dir = self.inner.tools_dir.join("cache");
        let inv = ytdlp::Invocation {
            video_id: &job.track.video_id,
            format: &format,
            ffmpeg: tools.ffmpeg.as_deref(),
            js_runtime: tools.js.as_ref().and_then(|j| j.ytdlp_arg()),
            cookies,
            cache_dir: &cache_dir,
            out_dir: &out_dir,
        };
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(ytdlp::args(&inv))
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Outcome::Failed(format!("yt-dlp liess sich nicht starten: {e}")),
        };
        let mut guard = GroupGuard(child.id());

        // stdout und stderr zeilenweise in einen Kanal (verlustfrei auch bei kaputtem UTF-8)
        let (tx, mut rx) = mpsc::channel::<(bool, String)>(64);
        for (is_err, stream) in [
            (false, child.stdout.take().map(Pipe::Out)),
            (true, child.stderr.take().map(Pipe::Err)),
        ] {
            if let Some(stream) = stream {
                tokio::spawn(read_lines(stream, is_err, tx.clone()));
            }
        }
        drop(tx);

        let mut errors = ytdlp::ErrorCapture::default();
        let mut last = Instant::now() - PROGRESS_EVERY;
        let mut progress = 0.0f32;
        loop {
            tokio::select! {
                line = rx.recv() => match line {
                    Some((true, line)) => errors.push(&line),
                    Some((false, line)) => {
                        if let Some(p) = ytdlp::parse_progress(&line) {
                            let stage_change = matches!(p, ytdlp::Progress::Postprocess { .. });
                            progress = ytdlp::overall(progress, p);
                            self.report(job, progress, &mut last, stage_change).await;
                        }
                    }
                    None => break,
                },
                _ = &mut cancel => {
                    guard.kill();
                    let _ = child.kill().await;
                    return Outcome::Canceled;
                }
            }
        }
        let status = tokio::select! {
            s = child.wait() => s,
            _ = &mut cancel => {
                guard.kill();
                let _ = child.kill().await;
                return Outcome::Canceled;
            }
        };
        // Normal beendet: die Gruppe gibt es nicht mehr
        guard.disarm();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                return Outcome::Failed(errors.message().unwrap_or_else(|| match s.code() {
                    Some(code) => format!("yt-dlp beendet mit Code {code}."),
                    None => "yt-dlp wurde beendet.".into(),
                }))
            }
            Err(e) => return Outcome::Failed(format!("yt-dlp: {e}")),
        }

        let Some(file) = find_output(&out_dir, &format).await else {
            return Outcome::Failed("yt-dlp hat keine Audiodatei erzeugt.".into());
        };
        let ext = audio_format(&file).unwrap_or_else(|| format.clone());
        self.report(job, progress.max(0.95), &mut last, true).await;

        // Tags und Cover
        let cover = match job.track.thumbnail.as_deref() {
            Some(url) => tagging::fetch_cover(&self.inner.http, url).await,
            None => None,
        };
        let data = tagging::TagData::from_track(&job.track);
        let tag_result = tokio::task::spawn_blocking({
            let file = file.clone();
            move || tagging::write_tags(&file, &data, cover)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        if let Err(e) = tag_result {
            return Outcome::Failed(format!("Tags nicht schreibbar: {e}"));
        }

        // Inzwischen abgebrochen? Dann nichts ablegen.
        if cancel.try_recv().is_ok() || !jobs::is_running(self.pool(), job.id).await.unwrap_or(true)
        {
            return Outcome::Canceled;
        }

        let parts = tagging::target_parts(&job.track, &ext);
        let path = match place(self.inner.library.root(), &parts, &file).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Download nicht ablegbar: {e}");
                return Outcome::Failed(
                    "Die Datei liess sich nicht im Musikordner ablegen.".into(),
                );
            }
        };
        match self.inner.library.scan_file(&path, true).await {
            Ok(outcome) => match outcome.id() {
                Some(id) => {
                    self.inner.library.notify_changed().await;
                    Outcome::Done(id)
                }
                None => {
                    Outcome::Failed("Die heruntergeladene Datei liess sich nicht lesen.".into())
                }
            },
            Err(e) => {
                tracing::warn!(file = %path.display(), "Download nicht lesbar: {e}");
                Outcome::Failed("Die heruntergeladene Datei liess sich nicht lesen.".into())
            }
        }
    }
}

/// Beendet beim Abbrechen (oder Herunterfahren) die ganze Prozessgruppe von yt-dlp – auch
/// ffmpeg und den Python-Prozess des eigenständigen yt-dlp.
struct GroupGuard(Option<u32>);

impl GroupGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }

    fn kill(&self) {
        #[cfg(unix)]
        if let Some(pid) = self
            .0
            .and_then(|p| i32::try_from(p).ok())
            .filter(|p| *p > 0)
        {
            // SAFETY: killpg schickt nur ein Signal an die eigene Prozessgruppe des Kindes
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

enum Pipe {
    Out(tokio::process::ChildStdout),
    Err(tokio::process::ChildStderr),
}

async fn read_lines(pipe: Pipe, is_err: bool, tx: mpsc::Sender<(bool, String)>) {
    let reader: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match pipe {
        Pipe::Out(s) => Box::new(s),
        Pipe::Err(s) => Box::new(s),
    };
    let mut reader = tokio::io::BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                // Ohne Empfänger weiterlesen, damit yt-dlp nicht an einer vollen Pipe hängt
                let _ = tx.send((is_err, line)).await;
            }
        }
    }
}

/// Die erzeugte Audiodatei im Ausgabeordner finden (bevorzugt mit der gewünschten Endung).
async fn find_output(dir: &Path, format: &str) -> Option<PathBuf> {
    let mut entries = tokio::fs::read_dir(dir).await.ok()?;
    let mut found = Vec::new();
    while let Ok(Some(e)) = entries.next_entry().await {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.ends_with(".part") || !path.is_file() {
            continue;
        }
        if let Some(ext) = audio_format(&path) {
            found.push((ext != format, path));
        }
    }
    found.sort();
    found.into_iter().next().map(|(_, p)| p)
}

/// Fertige Datei in den Musikordner legen: erst versteckt in den Zielordner kopieren (die
/// Überwachung ignoriert sie), dann unter dem ersten freien Namen sichtbar machen.
async fn place(root: &Path, parts: &[String], file: &Path) -> std::io::Result<PathBuf> {
    let (name, dirs) = parts.split_last().expect("mindestens ein Teil");
    let dir = prepare_dir(root, dirs).await?;
    let tmp = hidden_temp_name(&dir, "download");
    // Gleiches Dateisystem: umbenennen; sonst kopieren
    if tokio::fs::rename(file, &tmp).await.is_err() {
        if let Err(e) = tokio::fs::copy(file, &tmp).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    }
    publish_temp(&tmp, &dir, name).await
}
