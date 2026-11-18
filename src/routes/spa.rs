//! Die Oberfläche: eine Dioxus-App (WebAssembly, Quelltext in `ui/`), beim Übersetzen
//! eingebettet (siehe build.rs). Ihre Dateien liegen unter `/assets/`; jeder andere Pfad,
//! der nicht zur API gehört, bekommt `index.html` – den Rest erledigt der Router der App.

use rocket::http::{ContentType, Header, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder, Response};
use rocket::{get, routes, Request, Route};
use std::io::Cursor;
use std::path::PathBuf;

pub struct SpaFile {
    pub path: &'static str,
    pub body: &'static [u8],
    pub gzip: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/spa.rs"));

pub fn routes() -> Vec<Route> {
    routes![index, shared, fallback]
}

/// Ist die Oberfläche eingebettet? (Ohne `scripts/ui.sh` vor dem Übersetzen nicht.)
pub fn available() -> bool {
    FILES.iter().any(|f| f.path == "index.html")
}

fn find(path: &str) -> Option<&'static SpaFile> {
    FILES.iter().find(|f| f.path == path)
}

/// Versteht der Browser gzip?
pub struct AcceptsGzip(bool);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AcceptsGzip {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let gzip = req
            .headers()
            .get("Accept-Encoding")
            .any(|v| v.split(',').any(|e| e.trim().starts_with("gzip")));
        Outcome::Success(AcceptsGzip(gzip))
    }
}

pub enum Reply {
    /// Angepasste index.html (geteilte Links), nie zwischengespeichert
    Html(String),
    File(&'static SpaFile, bool),
    Missing,
}

impl<'r> Responder<'r, 'static> for Reply {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let (file, gzip) = match self {
            Reply::File(f, g) => (f, g),
            Reply::Html(body) => {
                return Response::build()
                    .header(ContentType::HTML)
                    .header(Header::new("Cache-Control", "no-cache"))
                    .sized_body(body.len(), Cursor::new(body))
                    .ok();
            }
            Reply::Missing => {
                let body = "<!doctype html><meta charset=utf-8><title>nrzMusic</title>\
                    <p style=\"font-family:sans-serif\">Die Oberfläche ist nicht eingebaut – vor dem \
                    Übersetzen <code>scripts/ui.sh</code> ausführen. Die API unter /api/v1 läuft.</p>";
                return Response::build()
                    .status(Status::ServiceUnavailable)
                    .header(ContentType::HTML)
                    .sized_body(body.len(), Cursor::new(body))
                    .ok();
            }
        };
        let ext = file.path.rsplit('.').next().unwrap_or("");
        let content_type = ContentType::from_extension(ext).unwrap_or(ContentType::Binary);
        // Dateien unter assets/ tragen eine Prüfsumme im Namen; index.html nie zwischenspeichern
        let cache = if file.path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        let mut res = Response::build();
        res.header(content_type)
            .header(Header::new("Cache-Control", cache))
            .header(Header::new("Vary", "Accept-Encoding"));
        match (gzip, file.gzip) {
            (true, Some(gz)) => res
                .header(Header::new("Content-Encoding", "gzip"))
                .sized_body(gz.len(), Cursor::new(gz)),
            _ => res.sized_body(file.body.len(), Cursor::new(file.body)),
        };
        res.ok()
    }
}

fn page(gzip: AcceptsGzip) -> Reply {
    find("index.html").map_or(Reply::Missing, |f| Reply::File(f, gzip.0))
}

#[get("/")]
fn index(gzip: AcceptsGzip) -> Reply {
    page(gzip)
}

/// Dateien der App oder – für alle Seiten der App – index.html. Niedriger Rang, damit API
/// und `/static` zuerst greifen; unbekannte API-Pfade bleiben 404 (JSON).
#[get("/<path..>", rank = 50)]
fn fallback(path: PathBuf, gzip: AcceptsGzip) -> Option<Reply> {
    let path = path.to_str()?.replace('\\', "/");
    if path.starts_with("api/") || path == "api" {
        return None;
    }
    if let Some(f) = find(&path) {
        return Some(Reply::File(f, gzip.0));
    }
    // Fehlende Datei (z. B. alte Prüfsumme nach einem Update) nicht als Seite beantworten
    if path.starts_with("assets/") || path.starts_with("static/") {
        return None;
    }
    Some(page(gzip))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Host der Anfrage (für absolute Adressen in der Link-Vorschau).
pub struct HostHeader(Option<String>, bool);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for HostHeader {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let host = req.headers().get_one("Host").map(str::to_string);
        let https = req.headers().get_one("X-Forwarded-Proto") == Some("https");
        Outcome::Success(HostHeader(host, https))
    }
}

/// Geteilter Link: dieselbe App, aber mit Titel und Cover für die Vorschau in Messengern
/// (Open Graph). Unbekannte Links bekommen die normale Seite – die App zeigt den Fehler.
#[get("/s/<token>")]
async fn shared(
    token: &str,
    gzip: AcceptsGzip,
    host: HostHeader,
    pool: &rocket::State<sqlx::SqlitePool>,
    config: &rocket::State<crate::config::Config>,
) -> Reply {
    let Some(file) = find("index.html") else { return Reply::Missing };
    let Ok(view) = crate::db::shares::view(pool, token, false).await else { return page(gzip) };
    let Ok(html) = std::str::from_utf8(file.body) else { return page(gzip) };
    let base = if !config.public_url.trim().is_empty() {
        config.public_url.trim().trim_end_matches('/').to_string()
    } else {
        match &host.0 {
            Some(h) => format!("{}://{h}", if host.1 { "https" } else { "http" }),
            None => String::new(),
        }
    };
    let title = match view.kind {
        nrzmusic_types::ShareKind::Favorites => format!("Likes von {}", view.owner),
        _ if view.title.is_empty() => "nrzMusic".to_string(),
        _ => view.title.clone(),
    };
    let desc = match view.kind {
        nrzmusic_types::ShareKind::Track => format!("{} · geteilt von {} auf nrzMusic", view.description, view.owner),
        _ => format!("{} Titel · geteilt von {} auf nrzMusic", view.tracks.len(), view.owner),
    };
    let mut meta = format!(
        "<meta property=\"og:type\" content=\"music.playlist\"><meta property=\"og:site_name\" content=\"nrzMusic\">\
         <meta property=\"og:title\" content=\"{t}\"><meta property=\"og:description\" content=\"{d}\">\
         <meta name=\"description\" content=\"{d}\"><meta name=\"twitter:card\" content=\"summary\">\
         <meta property=\"og:url\" content=\"{u}\">",
        t = escape(&title),
        d = escape(&desc),
        u = escape(&format!("{base}/s/{}", view.token)),
    );
    if let Some(img) = &view.artwork {
        let abs = if img.starts_with('/') { format!("{base}{img}") } else { img.clone() };
        meta.push_str(&format!("<meta property=\"og:image\" content=\"{}\">", escape(&abs)));
    }
    let body = html.replacen("</head>", &format!("{meta}</head>"), 1);
    Reply::Html(body)
}
