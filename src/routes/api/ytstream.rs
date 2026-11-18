//! `GET /api/v1/yt/stream/<video_id>`: die Tonspur eines YouTube-Titels für das
//! `<audio>`-Element (und die Android-App zum Speichern fürs Offline-Hören). Ist der Titel
//! schon heruntergeladen, kommt die Datei aus dem Musikordner; sonst löst yt-dlp die Adresse
//! auf (siehe `downloads::stream`) und die Bytes werden mit `Range` durchgereicht.

use super::media::{audio_content_type, AudioFile, RangeHeader};
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
    Remote(Proxied),
}

impl<'r> Responder<'r, 'static> for YtAudio {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        match self {
            YtAudio::File(f) => f.respond_to(req),
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

/// Tonspur vorab auflösen und warten, bis YouTube sie herausgibt (frische Adressen antworten
/// die ersten Sekunden mit 403). Die Oberfläche ruft das für den nächsten Titel auf, damit
/// er ohne Pause beginnt.
#[post("/yt/stream/<video_id>/prepare")]
pub async fn prepare(
    video_id: &str,
    settings: &State<SettingsStore>,
    downloads: &State<Downloads>,
) -> AppResult<Status> {
    if !settings.youtube_enabled() {
        return Err(AppError::YoutubeOff);
    }
    downloads.open_stream(video_id, Some("bytes=0-1")).await?;
    Ok(Status::NoContent)
}
