//! `GET /api/v1/yt/stream/<video_id>`: die Tonspur eines YouTube-Titels für das
//! `<audio>`-Element (und die Android-App zum Speichern fürs Offline-Hören). Ist der Titel
//! schon heruntergeladen, kommt die Datei aus dem Musikordner; sonst lädt der Server die
//! Tonspur ganz in seinen Zwischenspeicher (`downloads::audio_cache`) und liefert sie wie eine
//! Datei aus – stabil, mit bekannter Länge und Spulen ohne YouTube. Nur wenn das nicht klappt,
//! werden die Bytes von YouTube mit `Range` durchgereicht (`downloads::stream`).

use super::media::{audio_content_type, parse_range, AudioFile, ByteRange, RangeHeader};
use crate::downloads::audio_cache::{Fill, Inflight, Source};
use std::sync::Arc;
use crate::db::downloads as jobs;
use crate::db::library as store;
use crate::db::settings::SettingsStore;
use crate::downloads::Downloads;
use crate::error::{AppError, AppResult};
use crate::library::Library;
use futures::TryStreamExt;
use nrzmusic_types::DownloadStatus;
use rocket::http::{Header, Status};
use rocket::response::{self, Responder, Response};
use rocket::{get, post, routes, Request, Route, State};

pub fn routes() -> Vec<Route> {
    routes![stream, prepare]
}

/// Durchgereichte Antwort von YouTube.
pub struct Proxied {
    status: Status,
    headers: Vec<(&'static str, String)>,
    body: reqwest::Response,
}

impl<'r> Responder<'r, 'static> for Proxied {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut res = Response::build();
        res.status(self.status)
            .header(Header::new("Cache-Control", "no-store"))
            .header(Header::new("Accept-Ranges", "bytes"));
        for (k, v) in self.headers {
            res.header(Header::new(k, v));
        }
        let stream = self
            .body
            .bytes_stream()
            .map_err(|e| std::io::Error::other(e.to_string()));
        res.streamed_body(tokio_util::io::StreamReader::new(stream));
        res.ok()
    }
}

pub enum YtAudio {
    File(AudioFile),
    Growing(Growing),
    Remote(Proxied),
}

/// Antwort aus einer Tonspur, die gerade noch in den Zwischenspeicher lädt: Bereich und Länge
/// stehen fest, fehlende Bytes wartet der Body ab (siehe `downloads::audio_cache`).
pub struct Growing {
    range: ByteRange,
    total: u64,
    start: u64,
    len: u64,
    content_type: &'static str,
    file: tokio::fs::File,
    fill: tokio::sync::watch::Receiver<Fill>,
}

impl Growing {
    async fn open(job: Arc<Inflight>, total: u64, range: Option<&str>) -> std::io::Result<Self> {
        let range = parse_range(range, total);
        let (start, len) = match range {
            ByteRange::Full => (0, total),
            ByteRange::Partial(a, b) => (a, b - a + 1),
            ByteRange::Unsatisfiable => (0, 0),
        };
        // Jetzt öffnen: wird die Datei nach dem Laden umbenannt, bleibt der Zugriff gültig
        let file = tokio::fs::File::open(&job.part).await?;
        Ok(Growing { range, total, start, len, content_type: job.content_type(), file, fill: job.fill() })
    }

    /// Bytes `start..start+len` als Strom: liest, was schon da ist, und wartet auf den Rest.
    fn body(self) -> impl tokio::io::AsyncRead + Send {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let end = self.start + self.len;
        let state = (self.file, self.start, self.fill, false);
        let stream = futures::stream::unfold(state, move |(mut file, pos, mut fill, mut seeked)| async move {
            if pos >= end {
                return None;
            }
            loop {
                let f = *fill.borrow_and_update();
                if f.have > pos {
                    if !seeked {
                        if let Err(e) = file.seek(std::io::SeekFrom::Start(pos)).await {
                            return Some((Err(e), (file, end, fill, true)));
                        }
                        seeked = true;
                    }
                    let n = (f.have.min(end) - pos).min(64 * 1024) as usize;
                    let mut buf = vec![0u8; n];
                    return match file.read_exact(&mut buf).await {
                        Ok(_) => Some((Ok(bytes::Bytes::from(buf)), (file, pos + n as u64, fill, seeked))),
                        Err(e) => Some((Err(e), (file, end, fill, true))),
                    };
                }
                if f.failed {
                    return Some((Err(std::io::Error::other("Tonspur abgebrochen")), (file, end, fill, seeked)));
                }
                if fill.changed().await.is_err() {
                    // Ladevorgang beendet: was jetzt nicht da ist, kommt nicht mehr
                    let f = *fill.borrow();
                    if f.have <= pos {
                        return Some((Err(std::io::Error::other("Tonspur unvollständig")), (file, end, fill, seeked)));
                    }
                }
            }
        });
        tokio_util::io::StreamReader::new(stream)
    }
}

/// Body mit fester Länge für Rocket (`sized_body` verlangt `AsyncSeek`, gespult wird nie).
struct NoSeek<R>(R);

impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for NoSeek<R> {
    fn poll_read(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl<R: Unpin> tokio::io::AsyncSeek for NoSeek<R> {
    fn start_seek(self: std::pin::Pin<&mut Self>, position: std::io::SeekFrom) -> std::io::Result<()> {
        match position {
            std::io::SeekFrom::Current(0) | std::io::SeekFrom::Start(0) => Ok(()),
            _ => Err(std::io::Error::other("nicht spulbar")),
        }
    }
    fn poll_complete(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<std::io::Result<u64>> {
        std::task::Poll::Ready(Ok(0))
    }
}

impl<'r> Responder<'r, 'static> for Growing {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut res = Response::build();
        res.header(Header::new("Accept-Ranges", "bytes"))
            .header(Header::new("Cache-Control", "no-cache"))
            .header(Header::new("Content-Type", self.content_type));
        let (range, total, start, len) = (self.range, self.total, self.start, self.len);
        match range {
            ByteRange::Unsatisfiable => {
                res.status(Status::RangeNotSatisfiable)
                    .header(Header::new("Content-Range", format!("bytes */{total}")));
            }
            ByteRange::Partial(..) => {
                res.status(Status::PartialContent)
                    .header(Header::new("Content-Range", format!("bytes {start}-{}/{total}", start + len - 1)))
                    .sized_body(len as usize, NoSeek(Box::pin(self.body())));
            }
            ByteRange::Full => {
                res.status(Status::Ok).sized_body(len as usize, NoSeek(Box::pin(self.body())));
            }
        }
        res.ok()
    }
}

impl<'r> Responder<'r, 'static> for YtAudio {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        match self {
            YtAudio::File(f) => f.respond_to(req),
            YtAudio::Growing(g) => g.respond_to(req),
            YtAudio::Remote(p) => p.respond_to(req),
        }
    }
}

#[get("/yt/stream/<video_id>")]
pub async fn stream(
    video_id: &str,
    range: RangeHeader,
    settings: &State<SettingsStore>,
    downloads: &State<Downloads>,
    library: &State<Library>,
) -> AppResult<YtAudio> {
    if !settings.youtube_enabled() {
        return Err(AppError::YoutubeOff);
    }
    let range = range.0;

    // Schon im Musikordner? Dann die Datei.
    let done = jobs::by_video(library.pool(), video_id)
        .await?
        .filter(|j| j.status == DownloadStatus::Done)
        .and_then(|j| j.track_id);
    if let Some(track_id) = done {
        if let Some(t) = store::get(library.pool(), track_id).await? {
            let path = library.abs_path(&t.path);
            if path.is_file() {
                return Ok(YtAudio::File(
                    AudioFile::open(path, audio_content_type(&t.format), range.as_deref(), None)
                        .await?,
                ));
            }
        }
    }

    // Aus dem Zwischenspeicher: fertig (die nächsten Titel holt `prepare` schon vorab) oder
    // während des Ladens
    match downloads.audio_source(video_id, CACHE_WAIT).await {
        Ok(Source::Done(c)) => {
            return Ok(YtAudio::File(
                AudioFile::open(c.path, c.content_type, range.as_deref(), None).await?,
            ))
        }
        Ok(Source::Growing(job, total)) => match Growing::open(job, total, range.as_deref()).await {
            Ok(g) => return Ok(YtAudio::Growing(g)),
            Err(e) => tracing::info!(video = video_id, "Zwischenspeicher: {e} – reiche durch"),
        },
        Err(e) => tracing::info!(video = video_id, "Zwischenspeicher: {e} – reiche durch"),
    }

    let (resolved, resp) = downloads.open_stream(video_id, range.as_deref()).await?;

    let status = if resp.status().as_u16() == 206 {
        Status::PartialContent
    } else {
        Status::Ok
    };
    let mut headers: Vec<(&'static str, String)> =
        vec![("Content-Type", resolved.content_type.to_string())];
    for name in ["Content-Length", "Content-Range"] {
        if let Some(v) = resp.headers().get(name).and_then(|v| v.to_str().ok()) {
            headers.push((name, v.to_string()));
        }
    }
    Ok(YtAudio::Remote(Proxied {
        status,
        headers,
        body: resp,
    }))
}

/// So lange wartet die Wiedergabe höchstens auf die ganz geladene Tonspur.
const CACHE_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Tonspur vorab ganz in den Zwischenspeicher laden. Die Oberfläche ruft das für die nächsten
/// Titel auf, damit sie ohne Pause beginnen. Klappt das nicht, sagt die Prüfung bei YouTube
/// warum (410 = dauerhaft weg).
#[post("/yt/stream/<video_id>/prepare")]
pub async fn prepare(
    video_id: &str,
    settings: &State<SettingsStore>,
    downloads: &State<Downloads>,
) -> AppResult<Status> {
    if !settings.youtube_enabled() {
        return Err(AppError::YoutubeOff);
    }
    if downloads.cached_audio(video_id, std::time::Duration::from_secs(60)).await.is_ok() {
        return Ok(Status::NoContent);
    }
    downloads.open_stream(video_id, Some("bytes=0-1")).await?;
    Ok(Status::NoContent)
}
