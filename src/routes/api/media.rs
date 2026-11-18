//! Audio und Cover: Streaming mit `Range` (Spulen; Safari spielt Audio nur so), Download als
//! Anhang und Cover in passender Grösse.

use crate::db::library as store;
use crate::error::{AppError, AppResult};
use crate::library::{covers, Library};
use nrzmusic_types::ApiError;
use rocket::http::{ContentType, Header, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder, Response};
use rocket::{get, routes, Request, Route, State};
use std::io::{Cursor, SeekFrom};
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncSeek, AsyncSeekExt, ReadBuf};

pub fn routes() -> Vec<Route> {
    routes![stream, download, cover]
}

/// Content-Type zu einer Dateiendung (Track.format).
pub fn audio_content_type(format: &str) -> &'static str {
    match format {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        _ => "application/octet-stream",
    }
}

/// Ausgewerteter `Range`-Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    /// Kein (verwertbarer) Range-Header: ganze Datei, 200
    Full,
    /// Bytes `start..=end`, 206
    Partial(u64, u64),
    /// 416
    Unsatisfiable,
}

/// `bytes=a-b`, `bytes=a-`, `bytes=-n`. Andere Einheiten und mehrere Bereiche werden
/// ignoriert (ganze Datei – das erlaubt RFC 9110); kaputte oder unerfüllbare Angaben → 416.
pub fn parse_range(header: Option<&str>, size: u64) -> ByteRange {
    let Some(header) = header.map(str::trim) else {
        return ByteRange::Full;
    };
    let Some((unit, spec)) = header.split_once('=') else {
        return ByteRange::Unsatisfiable;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return ByteRange::Full;
    }
    if spec.contains(',') {
        return ByteRange::Full;
    }
    let Some((a, b)) = spec.trim().split_once('-') else {
        return ByteRange::Unsatisfiable;
    };
    let (a, b) = (a.trim(), b.trim());
    if size == 0 {
        return ByteRange::Unsatisfiable;
    }
    if a.is_empty() {
        // Die letzten n Bytes
        return match b.parse::<u64>() {
            Ok(n) if n > 0 => ByteRange::Partial(size - n.min(size), size - 1),
            _ => ByteRange::Unsatisfiable,
        };
    }
    let Ok(start) = a.parse::<u64>() else {
        return ByteRange::Unsatisfiable;
    };
    if start >= size {
        return ByteRange::Unsatisfiable;
    }
    if b.is_empty() {
        return ByteRange::Partial(start, size - 1);
    }
    match b.parse::<u64>() {
        Ok(end) if end >= start => ByteRange::Partial(start, end.min(size - 1)),
        _ => ByteRange::Unsatisfiable,
    }
}

/// Der rohe `Range`-Header der Anfrage.
pub struct RangeHeader(pub Option<String>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for RangeHeader {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        Outcome::Success(RangeHeader(
            req.headers().get_one("Range").map(String::from),
        ))
    }
}

/// Ein Abschnitt einer Datei als Body: liest nur `len` Bytes ab `start` und lässt sich
/// innerhalb des Abschnitts spulen (Rocket verlangt `AsyncSeek` für Bodies mit Grösse).
struct FileSlice {
    inner: tokio::io::Take<tokio::fs::File>,
    start: u64,
    len: u64,
}

impl AsyncRead for FileSlice {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl AsyncSeek for FileSlice {
    fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        let this = self.get_mut();
        let current = this.len - this.inner.limit();
        let target = match position {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::End(n) => this.len as i128 + n as i128,
            SeekFrom::Current(n) => current as i128 + n as i128,
        };
        let target = target.clamp(0, this.len as i128) as u64;
        Pin::new(this.inner.get_mut()).start_seek(SeekFrom::Start(this.start + target))
    }

    fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        let this = self.get_mut();
        match Pin::new(this.inner.get_mut()).poll_complete(cx) {
            Poll::Ready(Ok(abs)) => {
                let rel = abs.saturating_sub(this.start).min(this.len);
                this.inner.set_limit(this.len - rel);
                Poll::Ready(Ok(rel))
            }
            other => other,
        }
    }
}

/// Audiodatei als Antwort: 200 ganz, 206 Abschnitt, 416 unerfüllbar.
pub struct AudioFile {
    range: ByteRange,
    size: u64,
    body: Option<FileSlice>,
    content_type: &'static str,
    /// Als Anhang mit diesem Dateinamen (Download)
    attachment: Option<String>,
}

impl AudioFile {
    pub async fn open(
        path: PathBuf,
        content_type: &'static str,
        range: Option<&str>,
        attachment: Option<String>,
    ) -> AppResult<Self> {
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(AppError::NotFound),
            Err(e) => return Err(e.into()),
        };
        let size = file.metadata().await?.len();
        let range = parse_range(range, size);
        let body = match range {
            ByteRange::Full => Some((0, size)),
            ByteRange::Partial(start, end) => Some((start, end - start + 1)),
            ByteRange::Unsatisfiable => None,
        };
        let body = match body {
            Some((start, len)) => {
                if start > 0 {
                    file.seek(SeekFrom::Start(start)).await?;
                }
                Some(FileSlice {
                    inner: tokio::io::AsyncReadExt::take(file, len),
                    start,
                    len,
                })
            }
            None => None,
        };
        Ok(AudioFile {
            range,
            size,
            body,
            content_type,
            attachment,
        })
    }
}

/// `attachment; filename="…"; filename*=UTF-8''…` – ASCII-Ersatz plus der echte Name.
pub fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| match c {
            '"' | '\\' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'&'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

impl<'r> Responder<'r, 'static> for AudioFile {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut res = Response::build();
        res.header(Header::new("Accept-Ranges", "bytes"))
            .header(Header::new("Cache-Control", "no-cache"));
        if let Some(name) = &self.attachment {
            res.header(Header::new(
                "Content-Disposition",
                content_disposition(name),
            ));
        }
        match (self.range, self.body) {
            (ByteRange::Partial(start, end), Some(body)) => {
                res.status(Status::PartialContent)
                    .header(Header::new(
                        "Content-Range",
                        format!("bytes {start}-{end}/{}", self.size),
                    ))
                    .header(Header::new("Content-Type", self.content_type))
                    .max_chunk_size(64 * 1024)
                    .sized_body(body.len as usize, body);
            }
            (ByteRange::Full, Some(body)) => {
                res.status(Status::Ok)
                    .header(Header::new("Content-Type", self.content_type))
                    .max_chunk_size(64 * 1024)
                    .sized_body(body.len as usize, body);
            }
            _ => {
                let body = serde_json::to_string(&ApiError {
                    error: "Der angefragte Bereich liegt ausserhalb der Datei.".into(),
                })
                .unwrap_or_default();
                res.status(Status::RangeNotSatisfiable)
                    .header(Header::new(
                        "Content-Range",
                        format!("bytes */{}", self.size),
                    ))
                    .header(ContentType::JSON)
                    .sized_body(body.len(), Cursor::new(body));
            }
        }
        res.ok()
    }
}

#[get("/tracks/<id>/stream")]
async fn stream(id: i64, range: RangeHeader, library: &State<Library>) -> AppResult<AudioFile> {
    let track = store::get(library.pool(), id)
        .await?
        .ok_or(AppError::NotFound)?;
    AudioFile::open(
        library.abs_path(&track.path),
        audio_content_type(&track.format),
        range.0.as_deref(),
        None,
    )
    .await
}

#[get("/tracks/<id>/download")]
async fn download(id: i64, range: RangeHeader, library: &State<Library>) -> AppResult<AudioFile> {
    let track = store::get(library.pool(), id)
        .await?
        .ok_or(AppError::NotFound)?;
    let name = track
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&track.path)
        .to_string();
    AudioFile::open(
        library.abs_path(&track.path),
        audio_content_type(&track.format),
        range.0.as_deref(),
        Some(name),
    )
    .await
}

/// Cover-Bild; ändert sich nie (die ID ist der Hash des Inhalts).
pub struct CoverImage {
    data: Vec<u8>,
    content_type: &'static str,
}

impl<'r> Responder<'r, 'static> for CoverImage {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(Header::new("Content-Type", self.content_type))
            .header(Header::new(
                "Cache-Control",
                "public, max-age=31536000, immutable",
            ))
            .sized_body(self.data.len(), Cursor::new(self.data))
            .ok()
    }
}

/// `size` = Kantenlänge in Pixeln (verkleinert, JPEG), `0` oder ohne = Original.
#[get("/covers/<id>?<size>")]
async fn cover(id: &str, size: Option<u32>, library: &State<Library>) -> AppResult<CoverImage> {
    if !covers::valid_id(id) {
        return Err(AppError::NotFound);
    }
    let dir = library.covers_dir().to_path_buf();
    let size = size.unwrap_or(0);
    if size == 0 {
        return original_cover(&dir, id).await;
    }
    let (dir2, id2, size) = (dir.clone(), id.to_string(), covers::snap_size(size));
    let resized = tokio::task::spawn_blocking(move || covers::resized(&dir2, &id2, size))
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(e)))?;
    match resized {
        Ok(path) => Ok(CoverImage {
            data: tokio::fs::read(path).await?,
            content_type: "image/jpeg",
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(AppError::NotFound),
        Err(e) => {
            // Nicht verkleinerbar (exotisches Bild) – dann eben das Original
            tracing::warn!(cover = id, "Cover nicht verkleinerbar: {e}");
            original_cover(&dir, id).await
        }
    }
}

async fn original_cover(dir: &std::path::Path, id: &str) -> AppResult<CoverImage> {
    match tokio::fs::read(covers::original_path(dir, id)).await {
        Ok(data) => Ok(CoverImage {
            content_type: covers::content_type(&data),
            data,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(AppError::NotFound),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        use ByteRange::*;
        assert_eq!(parse_range(None, 100), Full);
        assert_eq!(parse_range(Some("bytes=0-1"), 100), Partial(0, 1));
        assert_eq!(parse_range(Some("bytes=10-"), 100), Partial(10, 99));
        assert_eq!(parse_range(Some("bytes=-5"), 100), Partial(95, 99));
        assert_eq!(parse_range(Some("bytes=-500"), 100), Partial(0, 99));
        assert_eq!(parse_range(Some("bytes=90-500"), 100), Partial(90, 99));
        assert_eq!(parse_range(Some("bytes=100-"), 100), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=5-2"), 100), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-0"), 100), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=abc"), 100), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=0-1"), 0), Unsatisfiable);
        assert_eq!(parse_range(Some("items=0-1"), 100), Full);
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 100), Full);
    }

    #[test]
    fn disposition() {
        assert_eq!(
            content_disposition("Bär \"x\".mp3"),
            "attachment; filename=\"B_r _x_.mp3\"; filename*=UTF-8''B%C3%A4r%20%22x%22.mp3"
        );
    }
}
