//! Gemeinsame Helfer der Integrationstests: Server mit leerem Daten- und Musikordner
//! (ohne Hintergrundarbeiten – gescannt wird im Test ausdrücklich), JSON-Aufrufe, kleine,
//! gültige Audiodateien (WAV, MP3 mit ID3v2) samt Cover-Bildern und ein kleiner HTTP-Server
//! mit vorbereiteten Antworten (Ersatz für Spotify und YouTubes Bild-Server).
#![allow(dead_code)]

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{Tag, TagType};
use nrzmusic::library::Library;
use rocket::figment::Figment;
use rocket::http::{Accept, ContentType, Status};
use rocket::local::asynchronous::Client;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Env {
    pub data: tempfile::TempDir,
    pub music: tempfile::TempDir,
    pub client: Client,
}

impl Env {
    pub fn music_path(&self, rel: &str) -> PathBuf {
        self.music.path().join(rel)
    }

    pub fn library(&self) -> &Library {
        self.client.rocket().state::<Library>().unwrap()
    }

    /// Voller Scan (wartet, bis er fertig ist).
    pub async fn scan(&self) -> nrzmusic::library::scanner::Changes {
        self.library().full_scan().await.unwrap().unwrap()
    }

    pub async fn call(
        &self,
        method: &str,
        uri: impl AsRef<str>,
        body: Option<Value>,
    ) -> (Status, Value) {
        call(&self.client, method, uri, body).await
    }

    pub async fn get(&self, uri: impl AsRef<str>) -> Value {
        let (status, v) = self.call("GET", uri.as_ref(), None).await;
        assert_eq!(status, Status::Ok, "GET {} → {v}", uri.as_ref());
        v
    }

    /// Datei in den Musikordner schreiben (Ordner werden angelegt).
    pub fn put_file(&self, rel: &str, data: &[u8]) -> PathBuf {
        let path = self.music_path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, data).unwrap();
        path
    }
}

pub fn figment(data: &Path, music: &Path) -> Figment {
    nrzmusic::config::figment()
        .merge(("data_dir", data))
        .merge(("music_dir", music))
        .merge(("log_level", "off"))
        .merge(("background", false))
        // Unabhängig davon, was auf dem Rechner installiert ist (Downloads-Tests setzen ein
        // Ersatz-yt-dlp)
        .merge(("ytdlp", "/nonexistent/nrzmusic-test/yt-dlp"))
        .merge(("ffmpeg", "/nonexistent/nrzmusic-test/ffmpeg"))
        .merge(("spotify_client_id", ""))
        // Die meisten Tests prüfen einen Nutzer ohne Anmeldung; tests/auth.rs schaltet Konten ein
        .merge(("auth", false))
}

pub async fn env_with(adjust: impl FnOnce(Figment) -> Figment) -> Env {
    env_with_services(adjust, nrzmusic::Services::default()).await
}

/// Server mit ausgetauschten Anbindungen (Ersatz-Spotify, Ersatz-Suche).
pub async fn env_with_services(
    adjust: impl FnOnce(Figment) -> Figment,
    services: nrzmusic::Services,
) -> Env {
    let data = tempfile::tempdir().unwrap();
    let music = tempfile::tempdir().unwrap();
    let rocket =
        nrzmusic::build_with_services(adjust(figment(data.path(), music.path())), services)
            .await
            .unwrap();
    let client = Client::tracked(rocket).await.unwrap();
    Env {
        data,
        music,
        client,
    }
}

pub async fn env() -> Env {
    env_with(|f| f).await
}

/// JSON-Aufruf.
pub async fn call(
    c: &Client,
    method: &str,
    uri: impl AsRef<str>,
    body: Option<Value>,
) -> (Status, Value) {
    let uri = uri.as_ref().to_string();
    let req = match method {
        "GET" => c.get(uri),
        "POST" => c.post(uri),
        "PUT" => c.put(uri),
        "PATCH" => c.patch(uri),
        "DELETE" => c.delete(uri),
        _ => unreachable!(),
    }
    .header(Accept::JSON);
    let req = match body {
        Some(b) => req.header(ContentType::JSON).body(b.to_string()),
        None => req,
    };
    let res = req.dispatch().await;
    let status = res.status();
    (
        status,
        res.into_json::<Value>().await.unwrap_or(Value::Null),
    )
}

// ---------------------------------------------------------------- Testdateien

/// Stille als WAV (8 kHz, mono, 16 Bit); `ms` bestimmt Länge und Grösse.
pub fn wav(ms: u32) -> Vec<u8> {
    let rate = 8000u32;
    let data_len = rate * ms / 1000 * 2;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend(b"RIFF");
    v.extend((36 + data_len).to_le_bytes());
    v.extend(b"WAVE");
    v.extend(b"fmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes()); // PCM
    v.extend(1u16.to_le_bytes()); // mono
    v.extend(rate.to_le_bytes());
    v.extend((rate * 2).to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend(data_len.to_le_bytes());
    v.resize(v.len() + data_len as usize, 0);
    v
}

/// Stille als MP3: `frames` gültige MPEG-1-Layer-III-Frames (128 kbit/s, 44,1 kHz, mono).
pub fn mp3(frames: usize) -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
    frame.repeat(frames)
}

/// Kleines PNG (einfarbig).
pub fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 40, 90]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

/// Kleines JPEG (einfarbig).
pub fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([20, 140, 90]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

#[derive(Default)]
pub struct Tags<'a> {
    pub title: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<&'a str>,
    pub picture: Option<Vec<u8>>,
}

/// MP3 mit ID3v2-Tag in den Musikordner schreiben.
pub fn put_tagged_mp3(env: &Env, rel: &str, frames: usize, tags: Tags<'_>) -> PathBuf {
    let path = env.put_file(rel, &mp3(frames));
    let mut tag = Tag::new(TagType::Id3v2);
    if let Some(v) = tags.title {
        tag.set_title(v.into());
    }
    if let Some(v) = tags.artist {
        tag.set_artist(v.into());
    }
    if let Some(v) = tags.album {
        tag.set_album(v.into());
    }
    if let Some(v) = tags.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, v.into());
    }
    if let Some(v) = tags.track {
        tag.set_track(v);
    }
    if let Some(v) = tags.disc {
        tag.set_disk(v);
    }
    if let Some(v) = tags.year {
        tag.set_year(v);
    }
    if let Some(v) = tags.genre {
        tag.set_genre(v.into());
    }
    if let Some(data) = tags.picture {
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            Some(MimeType::Png),
            None,
            data,
        ));
    }
    tag.save_to_path(&path, WriteOptions::default()).unwrap();
    path
}

/// Multipart-Body mit Feld `files` je Datei.
pub fn multipart(files: &[(&str, Vec<u8>)]) -> (ContentType, Vec<u8>) {
    let boundary = "nrzMusicBoundary7MA4YWxkTrZu0gW";
    let mut body = Vec::new();
    for (name, data) in files {
        body.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend(data);
        body.extend(b"\r\n");
    }
    body.extend(format!("--{boundary}--\r\n").as_bytes());
    (
        ContentType::new("multipart", "form-data").with_params(("boundary", boundary)),
        body,
    )
}

/// Titel per Pfad aus `GET /tracks` holen.
pub async fn track_by_path(env: &Env, path: &str) -> Value {
    let page = env.get("/api/v1/tracks?limit=1000").await;
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["path"] == path)
        .unwrap_or_else(|| panic!("{path} fehlt in {page}"))
        .clone()
}

/// Wartet, bis `check` etwas liefert (höchstens `secs` Sekunden).
pub async fn wait_for<T, F, Fut>(secs: u64, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if let Some(v) = check().await {
            return v;
        }
        assert!(
            std::time::Instant::now() < until,
            "Zeitlimit ({secs} s) überschritten"
        );
        rocket::tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------- Ersatz-HTTP-Server

#[derive(Debug, Clone)]
pub struct MockRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl MockRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Wert aus Query oder Formular-Body.
    pub fn param(&self, name: &str) -> Option<String> {
        [self.query.as_str(), self.body.as_str()]
            .iter()
            .flat_map(|s| s.split('&'))
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.replace('+', " "))
    }
}

pub struct MockResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl MockResponse {
    pub fn json(status: u16, v: Value) -> Self {
        MockResponse {
            status,
            content_type: "application/json",
            body: v.to_string().into_bytes(),
        }
    }

    pub fn bytes(content_type: &'static str, body: Vec<u8>) -> Self {
        MockResponse {
            status: 200,
            content_type,
            body,
        }
    }
}

pub type Handler = std::sync::Arc<dyn Fn(MockRequest) -> MockResponse + Send + Sync>;

/// Kleiner HTTP/1.1-Server auf 127.0.0.1 (eine Anfrage je Verbindung). Liefert die Basis-URL.
pub async fn mock_server(handler: Handler) -> String {
    use rocket::tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = rocket::tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    rocket::tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let handler = handler.clone();
            rocket::tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let header_end = loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
                let mut lines = head.split("\r\n");
                let mut first = lines.next().unwrap_or("").split(' ');
                let method = first.next().unwrap_or("").to_string();
                let target = first.next().unwrap_or("").to_string();
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                    .collect();
                let len: usize = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0);
                while buf.len() < header_end + len {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
                let (path, query) = match target.split_once('?') {
                    Some((p, q)) => (p.to_string(), q.to_string()),
                    None => (target.clone(), String::new()),
                };
                let res = handler(MockRequest {
                    method,
                    path,
                    query,
                    headers,
                    body,
                });
                let head = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    res.status,
                    res.content_type,
                    res.body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&res.body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}
